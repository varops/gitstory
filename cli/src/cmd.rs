//! Command implementations. Each returns JSON data plus human text.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::PathBuf;

use gitstory_proto::evidence::{EventType, EvidenceEvent};
use gitstory_proto::id;
use gitstory_proto::record::{
    EgressManifest, LinkEntry, LinkKind, LinkOp, LinkProposal, ProposalStatus,
};
use gitstory_proto::story::{Claim, ClaimStatus, ClaimType, Stage, Story, StoryStatus};
use serde_json::{Value, json};

use crate::adapters::{self, Ctx};
use crate::collect::{self, WorkUnit};
use crate::config::{self, RepoConfig};
use crate::error::{Context, Result};
use crate::git::Repo;
use crate::hooks;
use crate::index::{self, Index};
use crate::model::{self, Gate};
use crate::render;
use crate::synth;
use crate::workspace::{self, Workspace};
use crate::{err, fail};

pub struct Out {
    pub data: Value,
    pub human: String,
}

impl Out {
    fn new(data: Value, human: impl Into<String>) -> Self {
        Self {
            data,
            human: human.into(),
        }
    }
}

pub struct Env {
    pub cwd: PathBuf,
    pub brief: bool,
}

impl Env {
    fn ws(&self) -> Result<Workspace> {
        Workspace::open(&self.cwd)
    }
}

fn story_view(env: &Env, s: &Story) -> Value {
    if env.brief {
        render::brief(s)
    } else {
        serde_json::to_value(s).expect("story serializes")
    }
}

// ---------------------------------------------------------------------------
// init, trust

pub fn init(env: &Env, repository: Option<String>, trust: bool, force: bool) -> Result<Out> {
    let repo = Repo::discover(&env.cwd)?;
    let path = repo.root.join(config::FILE_NAME);
    let existing = RepoConfig::load(&repo.root)?;
    let (cfg, created) = match existing {
        Some(c) if !force => {
            if repository.is_some() {
                fail!(
                    Config,
                    "{} already exists; pass --force to overwrite it",
                    path.display()
                );
            }
            (c, false)
        }
        _ => {
            let cfg = RepoConfig::new(repository);
            let text = format!(
                "# GitStory configuration. Safe to commit: holds no credentials.\n# Raw transcripts never leave this machine; see [egress].\n{}",
                cfg.to_toml()
            );
            fs::write(&path, text).context(|| format!("write {}", path.display()))?;
            (cfg, true)
        }
    };
    let store = workspace::open_store(&repo, &cfg)?;
    let (endpoint, local) = model::endpoint(&cfg.model);
    if trust && !local {
        config::trust(&endpoint)?;
    }
    let trusted = local || config::trusted(&endpoint)?;
    let mut human = if created {
        format!("wrote {}\n", path.display())
    } else {
        format!("{} already present\n", path.display())
    };
    human.push_str(&format!(
        "store: {} (repository {})\n",
        store.backend.describe(),
        store.repo
    ));
    human.push_str(&format!(
        "model: {} {} at {}{}",
        cfg.model.provider,
        cfg.model.name,
        endpoint,
        if trusted {
            ""
        } else {
            " (not trusted yet: run `git story trust`)"
        }
    ));
    Ok(Out::new(
        json!({
            "path": path,
            "created": created,
            "repository": store.repo,
            "store": store.backend.describe(),
            "model": {"provider": cfg.model.provider, "name": cfg.model.name, "endpoint": endpoint, "local": local, "trusted": trusted},
            "egress": cfg.egress,
        }),
        human,
    ))
}

pub fn trust(env: &Env, endpoint: Option<String>) -> Result<Out> {
    let targets = match endpoint {
        Some(e) => vec![e],
        None => {
            let ws = env.ws()?;
            let mut v = vec![model::endpoint(&ws.cfg.model)];
            if let Some(m) = &ws.cfg.validation
                && m.provider != "none"
            {
                v.push(model::endpoint(m));
            }
            v.into_iter()
                .filter(|(_, local)| !local)
                .map(|(e, _)| e)
                .collect()
        }
    };
    for t in &targets {
        config::trust(t)?;
    }
    Ok(Out::new(
        json!({"trusted": targets}),
        if targets.is_empty() {
            "all configured endpoints are local; nothing to trust".to_owned()
        } else {
            format!("trusted {}", targets.join(", "))
        },
    ))
}

// ---------------------------------------------------------------------------
// notes and collection

/// Narrow session evidence to the task that produced the change: the last
/// request before the first session edit to a changed path starts it.
/// Earlier session events belong to other tasks in the same session.
fn focus(mut events: Vec<EvidenceEvent>, changed: &BTreeSet<String>) -> Vec<EvidenceEvent> {
    let session =
        |e: &EvidenceEvent| e.durable.source == gitstory_proto::evidence::SourceType::AgentSession;
    let Some(first_edit) = events
        .iter()
        .filter(|e| {
            session(e)
                && e.kind == EventType::Attempt
                && e.paths.iter().any(|p| changed.contains(p))
        })
        .map(|e| e.durable.at)
        .min()
    else {
        return events;
    };
    let Some(start) = events
        .iter()
        .filter(|e| session(e) && e.kind == EventType::Goal && e.durable.at <= first_edit)
        .map(|e| e.durable.at)
        .max()
    else {
        return events;
    };
    events.retain(|e| !session(e) || e.durable.at >= start);
    // The request that started it is the goal; later ones are follow-ups.
    events
}

fn work_unit_arg(ws: &Workspace, w: Option<String>) -> String {
    w.unwrap_or_else(|| collect::default_work_unit(ws))
}

pub fn note(
    env: &Env,
    text: Option<String>,
    kind: Option<&str>,
    supersedes: Option<String>,
    test: Option<(String, bool)>,
    work_unit: Option<String>,
) -> Result<Out> {
    let ws = env.ws()?;
    let wu_id = work_unit_arg(&ws, work_unit);
    let mut wu = WorkUnit::open(&ws, &wu_id)?;
    let ctx = Ctx::new(&ws, &wu_id);
    let ev = match (test, text) {
        (Some((cmd, passed)), _) => adapters::test_note(&ctx, &cmd, passed),
        (None, Some(t)) => {
            let k = match kind.unwrap_or("decision") {
                "reason" => EventType::Reason,
                "constraint" => EventType::Constraint,
                _ => EventType::Decision,
            };
            adapters::note(&ctx, k, &t)
        }
        (None, None) => fail!(
            Usage,
            "give the note text, or --test <cmd> --result pass|fail"
        ),
    };
    if let Some(sid) = &supersedes {
        let idx = ws.index()?;
        let full = resolve_story(&idx, sid)?;
        if !wu.meta.supersedes.contains(&full) {
            wu.meta.supersedes.push(full);
            wu.save_meta()?;
        }
    }
    wu.add_events(std::slice::from_ref(&ev))?;
    Ok(Out::new(
        json!({"work_unit": wu_id, "event": ev, "supersedes": wu.meta.supersedes}),
        format!("noted {} in work unit {wu_id}", ev.id),
    ))
}

/// Run every enabled adapter for the work unit. Returns new event count.
fn gather(ws: &Workspace, wu: &mut WorkUnit) -> Result<usize> {
    let wid = wu.meta.work_unit.clone();
    let ctx = Ctx::new(ws, &wid);
    let mut events: Vec<EvidenceEvent> = adapters::git(&ctx, wu)?;
    if ws.cfg.collection.claude_code {
        events.extend(adapters::claude_code(&ctx, wu)?);
    }
    if ws.cfg.collection.ghf {
        events.extend(crate::ghf::traces(&ctx, wu)?);
    }
    let n = wu.add_events(&events)?;
    wu.save_meta()?;
    Ok(n)
}

fn ignored_fn<'a>(ws: &'a Workspace, wu_id: &'a str) -> impl Fn(&str) -> bool + 'a {
    let ctx = Ctx::new(ws, wu_id);
    move |p: &str| ctx.ignored(p)
}

pub fn draft(env: &Env, work_unit: Option<String>, background: bool) -> Result<Out> {
    let ws = env.ws()?;
    let wu_id = work_unit_arg(&ws, work_unit);
    if background {
        spawn_background(&ws, &wu_id)?;
        return Ok(Out::new(
            json!({"work_unit": wu_id, "background": true}),
            format!("drafting {wu_id} in the background"),
        ));
    }
    let mut wu = WorkUnit::open(&ws, &wu_id)?;
    let new = gather(&ws, &mut wu)?;
    let head = ws
        .repo
        .head()
        .ok_or_else(|| err!(NoEvidence, "the repository has no commits yet"))?;
    let commit = ws.repo.commit(&head)?;
    let diff = match &wu.meta.base_rev {
        Some(base) => ws.repo.worktree_diff(base)?,
        None => ws.repo.commit_diff(&head)?,
    };
    let changed: BTreeSet<String> = diff
        .iter()
        .flat_map(|f| std::iter::once(f.path.clone()).chain(f.old_path.clone()))
        .collect();
    let events = focus(wu.events()?, &changed);
    let mut gate = Gate::new(&ws.cfg.egress);
    let ignored = ignored_fn(&ws, &wu_id);
    let input = synth::Input {
        work_unit: &wu_id,
        repo: &ws.store.repo,
        commit: &head,
        commit_subject: &commit.subject,
        events: &events,
        diff: &diff,
        drafts: &[],
        stage: Stage::Draft,
        supersedes: wu.meta.supersedes.clone(),
    };
    let egress_id = id::new_id(id::EGRESS);
    let result = synth::synthesize(&ws.cfg, &mut gate, &input, &ignored)
        .and_then(|s| synth::validate(&ws.cfg, &mut gate, s, &events, &diff, &ignored));
    // The manifest is kept even when the request failed: data may have left.
    let manifest = EgressManifest {
        id: egress_id.clone(),
        calls: gate.calls,
    };
    wu.put_egress(&manifest)?;
    let mut story = result?;
    story.egress = egress_id;
    wu.put_draft(&story)?;
    Ok(Out::new(
        json!({"story": story_view(env, &story), "work_unit": wu_id, "new_events": new, "events": events.len(), "egress": manifest}),
        render::markdown(&story),
    ))
}

fn spawn_background(ws: &Workspace, wu: &str) -> Result<()> {
    if std::env::var_os("GITSTORY_BACKGROUND").is_some() {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let log_dir = ws.repo.common_dir.join("story");
    fs::create_dir_all(&log_dir)?;
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("draft.log"))?;
    let err_log = log.try_clone()?;
    let mut c = std::process::Command::new(exe);
    c.args(["draft", "--work-unit", wu, "--json"])
        .current_dir(&ws.repo.root)
        .env("GITSTORY_BACKGROUND", "1")
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(err_log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    c.spawn()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// land

pub fn land(
    env: &Env,
    work_unit: Option<String>,
    to: String,
    since: Option<String>,
) -> Result<Out> {
    let ws = env.ws()?;
    let wu_id = work_unit_arg(&ws, work_unit);
    let sha = ws.require_commit(&to)?;
    let main = ws.repo.main_ref(ws.cfg.main_ref()).ok_or_else(|| {
        err!(
            Config,
            "cannot find the main branch; set [git] main in .gitstory"
        )
    })?;
    if !ws.repo.is_ancestor(&sha, &main) {
        fail!(
            CommitNotReachable,
            "commit {sha} is not reachable from {main}; GitStory binds stories to landed commits only"
        );
    }
    let mut wu = WorkUnit::open(&ws, &wu_id)?;
    let commit = ws.repo.commit(&sha)?;
    // A run of commits pushed straight to main lands as one story.
    let since = match &since {
        Some(r) => {
            let base = ws.require_commit(r)?;
            if base == sha || !ws.repo.is_ancestor(&base, &sha) {
                fail!(Usage, "--since {r} must be an ancestor of {to}");
            }
            let start = ws.repo.commit(&base)?.time * 1000;
            wu.meta.base_rev = Some(base.clone());
            wu.meta.tip = Some(sha.clone());
            wu.meta.window = Some((start, commit.time * 1000 + 10 * 60_000));
            Some(base)
        }
        None => None,
    };
    gather(&ws, &mut wu)?;
    let drafts = wu.drafts()?;
    let diff = match &since {
        Some(base) => ws.repo.range_diff(base, &sha)?,
        None => ws.repo.commit_diff(&sha)?,
    };
    let landed_paths: BTreeSet<String> = diff
        .iter()
        .flat_map(|f| std::iter::once(f.path.clone()).chain(f.old_path.clone()))
        .collect();
    let events = focus(wu.events()?, &landed_paths);
    let mut gate = Gate::new(&ws.cfg.egress);
    let ignored = ignored_fn(&ws, &wu_id);
    let input = synth::Input {
        work_unit: &wu_id,
        repo: &ws.store.repo,
        commit: &sha,
        commit_subject: &commit.subject,
        events: &events,
        diff: &diff,
        drafts: &drafts,
        stage: Stage::Landed,
        supersedes: wu.meta.supersedes.clone(),
    };
    let egress_id = id::new_id(id::EGRESS);
    let result = synth::synthesize(&ws.cfg, &mut gate, &input, &ignored)
        .map(|s| synth::guard_landed(s, &events, &landed_paths))
        .and_then(|s| synth::validate(&ws.cfg, &mut gate, s, &events, &diff, &ignored));
    let manifest = EgressManifest {
        id: egress_id.clone(),
        calls: gate.calls,
    };
    let mut story = match result {
        Ok(s) => s,
        Err(f) => {
            wu.put_egress(&manifest)?;
            return Err(f);
        }
    };
    story.egress = egress_id;
    story.contains = match &since {
        Some(base) => ws
            .repo
            .commits_between(base, &sha)?
            .into_iter()
            .filter(|c| *c != sha)
            .collect(),
        None => ws.repo.merged_commits(&sha)?,
    };
    // Only the evidence the story cites goes to the shared store.
    let cited: HashSet<&str> = story
        .claims()
        .into_iter()
        .flat_map(|(_, c)| c.evidence.iter().map(String::as_str))
        .collect();
    for e in events.iter().filter(|e| cited.contains(e.id.as_str())) {
        ws.store.put_event(e)?;
    }
    ws.store.put_egress(&manifest)?;
    // Explicit supersedes from notes become links, after a cycle check.
    let author = ws.author();
    let mut graph = index::link_graph(&ws.store)?;
    for old in &story.supersedes {
        if creates_cycle(&graph, &story.story_id, old) {
            continue;
        }
        graph
            .supersedes
            .insert((story.story_id.clone(), old.clone()), ());
    }
    ws.store.put_story(&story)?;
    for old in &story.supersedes {
        ws.store.append_link(&LinkEntry {
            op: LinkOp::Add,
            kind: LinkKind::Supersedes,
            from: story.story_id.clone(),
            to: old.clone(),
            at: id::now_ms(),
            author: author.clone(),
            proposal: None,
        })?;
    }
    let blame_base = match &since {
        Some(base) => vec![base.clone()],
        None => commit.parents.clone(),
    };
    let proposals = propose_links(&ws, &story, &diff, &blame_base)?;
    crate::ghf::release(&Ctx::new(&ws, &wu_id), &mut wu);
    wu.meta.landed.push((sha.clone(), story.story_id.clone()));
    wu.meta.landed_at = Some(id::now_ms());
    wu.save_meta()?;
    let mut human = render::markdown(&story);
    for p in &proposals {
        human.push_str(&format!(
            "\nproposed: {} {:?} {} ({}) — accept with `git story link --accept {}`",
            p.from, p.kind, p.to, p.reason, p.id
        ));
    }
    Ok(Out::new(
        json!({"story": story_view(env, &story), "proposals": proposals, "cited_events": cited.len(), "egress": manifest}),
        human,
    ))
}

/// Blame the lines the landed commit changed (in its parent) and propose
/// `supersedes` links to the stories behind them. Never applied on its own.
fn propose_links(
    ws: &Workspace,
    story: &Story,
    diff: &[crate::git::FileDiff],
    parents: &[String],
) -> Result<Vec<LinkProposal>> {
    let Some(parent) = parents.first() else {
        return Ok(Vec::new());
    };
    let idx = ws.index()?;
    let existing: HashSet<(String, String)> = ws
        .store
        .proposals()?
        .into_iter()
        .map(|p| (p.from, p.to))
        .collect();
    let mut found: BTreeMap<String, String> = BTreeMap::new();
    for f in diff {
        let path = f.old_path.as_deref().unwrap_or(&f.path);
        for (start, count) in &f.old_ranges {
            let end = start + count.saturating_sub(1);
            let Ok(shas) = ws.repo.blame(parent, path, Some((*start, end))) else {
                continue;
            };
            for old in idx.for_commits(&shas.into_iter().collect::<Vec<_>>())? {
                if old.story_id != story.story_id && old.stage == Stage::Landed {
                    found.entry(old.story_id.clone()).or_insert_with(|| {
                        format!(
                            "changes {path}:{start}-{end}, last touched by {}",
                            &old.commit[..old.commit.len().min(10)]
                        )
                    });
                }
            }
        }
    }
    let mut out = Vec::new();
    for (old, reason) in found {
        if story.supersedes.contains(&old)
            || existing.contains(&(story.story_id.clone(), old.clone()))
        {
            continue;
        }
        let p = LinkProposal {
            id: id::new_id(id::PROPOSAL),
            kind: LinkKind::Supersedes,
            from: story.story_id.clone(),
            to: old,
            reason,
            status: ProposalStatus::Open,
            at: id::now_ms(),
        };
        ws.store.put_proposal(&p)?;
        out.push(p);
    }
    Ok(out)
}

/// Would `from supersedes to` close a loop?
fn creates_cycle(g: &index::LinkGraph, from: &str, to: &str) -> bool {
    if from == to {
        return true;
    }
    // Follow "supersedes" edges from `to`; reaching `from` means a cycle.
    let mut stack = vec![to.to_owned()];
    let mut seen = HashSet::new();
    while let Some(n) = stack.pop() {
        if n == from {
            return true;
        }
        if !seen.insert(n.clone()) {
            continue;
        }
        for (a, b) in g.supersedes.keys() {
            if *a == n {
                stack.push(b.clone());
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// retrieval

fn resolve_story(idx: &Index, r: &str) -> Result<String> {
    let prefix = if r.starts_with("st_") {
        r.to_owned()
    } else {
        format!("st_{r}")
    };
    match idx.ids_with_prefix(&prefix)?.as_slice() {
        [id] => Ok(id.clone()),
        [] => fail!(NotFound, "no story matches {r:?}"),
        _ => fail!(Ambiguous, "{r:?} matches more than one story"),
    }
}

/// Drafts on this machine attached to `sha`.
fn drafts_for(ws: &Workspace, sha: &str) -> Result<Vec<Story>> {
    let mut out = Vec::new();
    for wu in WorkUnit::all(ws)? {
        if let Some(d) = wu.drafts()?.into_iter().rfind(|d| d.commit == sha) {
            out.push(d);
        }
    }
    Ok(out)
}

pub fn show(env: &Env, rev: Option<String>) -> Result<Out> {
    let ws = env.ws()?;
    let rev = rev.unwrap_or_else(|| "HEAD".into());
    let idx = ws.index()?;
    // A story id works as well as a commit.
    if rev.starts_with("st_")
        && let Ok(id) = resolve_story(&idx, &rev)
        && let Some(s) = idx.story(&id)?
    {
        return Ok(Out::new(
            json!({"stories": [story_view(env, &s)]}),
            render::markdown(&s),
        ));
    }
    let sha = ws.require_commit(&rev)?;
    let mut stories = idx.for_commits(std::slice::from_ref(&sha))?;
    if stories.is_empty() {
        stories = drafts_for(&ws, &sha)?;
    }
    if stories.is_empty() {
        fail!(
            NotFound,
            "no story for {rev} ({})",
            &sha[..10.min(sha.len())]
        );
    }
    let human = stories
        .iter()
        .map(render::markdown)
        .collect::<Vec<_>>()
        .join("\n---\n\n");
    Ok(Out::new(
        json!({"commit": sha, "stories": stories.iter().map(|s| story_view(env, s)).collect::<Vec<_>>()}),
        human,
    ))
}

/// Follow `superseded_by` links to the stories that hold now.
fn current_of(idx: &Index, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![id.to_owned()];
    let mut seen = HashSet::new();
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let next = idx.successors(&n);
        if next.is_empty() {
            if n != id {
                out.push(n);
            }
        } else {
            stack.extend(next);
        }
    }
    out
}

pub fn blame(env: &Env, target: String) -> Result<Out> {
    let ws = env.ws()?;
    let (path, range) = match target.rsplit_once(':') {
        Some((p, r)) if r.chars().all(|c| c.is_ascii_digit() || c == '-') && !r.is_empty() => {
            let (a, b) = match r.split_once('-') {
                Some((a, b)) => (a.parse().unwrap_or(1), b.parse().unwrap_or(1)),
                None => {
                    let n: u32 = r.parse().unwrap_or(1);
                    (n, n)
                }
            };
            (p.to_owned(), Some((a.min(b).max(1), a.max(b).max(1))))
        }
        _ => (target.clone(), None),
    };
    let idx = ws.index()?;
    let shas: Vec<String> = ws.repo.blame("HEAD", &path, range)?.into_iter().collect();
    let found = idx.for_commits(&shas)?;
    let mut entries = Vec::new();
    let mut listed = HashSet::new();
    let mut human = Vec::new();
    for s in &found {
        if !listed.insert(s.story_id.clone()) {
            continue;
        }
        let current = current_of(&idx, &s.story_id);
        if s.status == StoryStatus::Current || !env.brief {
            entries.push(json!({"story": story_view(env, s), "current": s.status == StoryStatus::Current, "via": "blame"}));
            human.push(render::line(s));
        }
        for c in current {
            if listed.insert(c.clone())
                && let Some(cs) = idx.story(&c)?
            {
                entries.push(
                    json!({"story": story_view(env, &cs), "current": true, "via": s.story_id}),
                );
                human.push(format!(
                    "{}   (current; supersedes {})",
                    render::line(&cs),
                    s.story_id
                ));
            }
        }
    }
    if human.is_empty() {
        human.push(format!("no stories behind {target}"));
    }
    Ok(Out::new(
        json!({"path": path, "range": range, "commits": shas, "stories": entries}),
        human.join("\n"),
    ))
}

pub fn path(env: &Env, p: String) -> Result<Out> {
    let ws = env.ws()?;
    let idx = ws.index()?;
    let mut stories = idx.for_path(&p)?;
    // Current decisions first, newest first within each group.
    stories.sort_by_key(|s| {
        (
            s.status != StoryStatus::Current,
            std::cmp::Reverse(s.created_at),
        )
    });
    let shown: Vec<&Story> = stories
        .iter()
        .filter(|s| !env.brief || s.status == StoryStatus::Current)
        .collect();
    let human = if shown.is_empty() {
        format!("no stories touch {p}")
    } else {
        shown
            .iter()
            .map(|s| render::line(s))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(Out::new(
        json!({"path": p, "stories": shown.iter().map(|s| story_view(env, s)).collect::<Vec<_>>()}),
        human,
    ))
}

pub fn history(env: &Env, r: String) -> Result<Out> {
    let ws = env.ws()?;
    let idx = ws.index()?;
    let id = resolve_story(&idx, &r)?;
    let mut all = BTreeSet::new();
    let mut stack = vec![id.clone()];
    while let Some(n) = stack.pop() {
        if !all.insert(n.clone()) {
            continue;
        }
        stack.extend(idx.predecessors(&n));
        stack.extend(idx.successors(&n));
    }
    let mut chain: Vec<Story> = all
        .iter()
        .filter_map(|i| idx.story(i).ok().flatten())
        .collect();
    chain.sort_by_key(|s| s.created_at);
    let human = chain
        .iter()
        .map(render::line)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Out::new(
        json!({"story": id, "chain": chain.iter().map(|s| story_view(env, s)).collect::<Vec<_>>()}),
        human,
    ))
}

pub fn search(env: &Env, query: String) -> Result<Out> {
    let ws = env.ws()?;
    let idx = ws.index()?;
    let mut hits = idx.search(&query, 20)?;
    hits.sort_by_key(|s| s.status != StoryStatus::Current);
    let human = if hits.is_empty() {
        "no matches".to_owned()
    } else {
        hits.iter().map(render::line).collect::<Vec<_>>().join("\n")
    };
    Ok(Out::new(
        json!({"query": query, "stories": hits.iter().map(|s| story_view(env, s)).collect::<Vec<_>>()}),
        human,
    ))
}

/// Re-run validation on a landed story using only what the store holds:
/// the durable parts of cited events. Session logs, CI links and ghf traces
/// may be gone; the story must still hold up (PRD §8.4).
pub fn validate(env: &Env, rev: String) -> Result<Out> {
    let ws = env.ws()?;
    let idx = ws.index()?;
    let id = if rev.starts_with("st_") {
        resolve_story(&idx, &rev)?
    } else {
        let sha = ws.require_commit(&rev)?;
        match idx.for_commits(std::slice::from_ref(&sha))?.as_slice() {
            [one] => one.story_id.clone(),
            [] => fail!(NotFound, "no landed story for {rev}"),
            _ => fail!(Ambiguous, "{rev} has several stories; pass a story id"),
        }
    };
    let s = ws
        .store
        .story(&id)?
        .ok_or_else(|| err!(NotFound, "no story {id}"))?;
    let mut events = Vec::new();
    let mut missing = Vec::new();
    for e in s
        .claims()
        .into_iter()
        .flat_map(|(_, c)| c.evidence.clone())
        .collect::<BTreeSet<_>>()
    {
        match ws.store.event(&e)? {
            Some(ev) => events.push(ev),
            None => missing.push(e),
        }
    }
    // A story landed as a run of commits covers their whole range.
    let single_parent = ws
        .repo
        .commit(&s.commit)
        .is_ok_and(|c| c.parents.len() == 1);
    let diff = match s.contains.first() {
        Some(oldest) if single_parent => ws.repo.range_diff(&format!("{oldest}^"), &s.commit),
        _ => ws.repo.commit_diff(&s.commit),
    }
    .unwrap_or_default();
    let ignored = ignored_fn(&ws, &s.work_unit);
    let mut gate = Gate::new(&ws.cfg.egress);
    let checked = synth::validate(&ws.cfg, &mut gate, s.clone(), &events, &diff, &ignored)?;
    let manifest = EgressManifest {
        id: id::new_id(id::EGRESS),
        calls: gate.calls,
    };
    if !manifest.calls.is_empty() {
        ws.store.put_egress(&manifest)?;
    }
    let kept: HashSet<&str> = checked
        .decisions
        .iter()
        .chain(&checked.rejected)
        .chain(&checked.discarded)
        .chain(&checked.evidence_summary)
        .map(|c| c.claim.as_str())
        .collect();
    let failed: Vec<&str> = s
        .decisions
        .iter()
        .chain(&s.rejected)
        .chain(&s.discarded)
        .chain(&s.evidence_summary)
        .map(|c| c.claim.as_str())
        .filter(|c| !kept.contains(c))
        .collect();
    let ok = failed.is_empty() && missing.is_empty();
    let mut human = format!(
        "{id}: {}",
        if ok {
            "every authoritative claim holds"
        } else {
            "validation failed"
        }
    );
    for f in &failed {
        human.push_str(&format!("\n  unsupported: {f}"));
    }
    for m in &missing {
        human.push_str(&format!("\n  missing evidence: {m}"));
    }
    Ok(Out::new(
        json!({"story": id, "valid": ok, "unsupported": failed, "missing_events": missing, "egress": manifest}),
        human,
    ))
}

// ---------------------------------------------------------------------------
// links

pub fn link(
    env: &Env,
    story: Option<String>,
    supersedes: Option<String>,
    related: Option<String>,
    accept: Option<String>,
    reject: Option<String>,
    remove: bool,
) -> Result<Out> {
    let ws = env.ws()?;
    let author = ws.author();
    if let Some(pid) = accept.as_ref().or(reject.as_ref()) {
        let mut p = ws
            .store
            .proposal(pid)?
            .ok_or_else(|| err!(NotFound, "no proposal {pid}"))?;
        if p.status != ProposalStatus::Open {
            fail!(Usage, "proposal {pid} is already {:?}", p.status);
        }
        if accept.is_some() {
            let g = index::link_graph(&ws.store)?;
            if p.kind == LinkKind::Supersedes && creates_cycle(&g, &p.from, &p.to) {
                fail!(
                    LinkCycle,
                    "accepting {pid} would create a supersession cycle"
                );
            }
            ws.store.append_link(&LinkEntry {
                op: LinkOp::Add,
                kind: p.kind,
                from: p.from.clone(),
                to: p.to.clone(),
                at: id::now_ms(),
                author,
                proposal: Some(p.id.clone()),
            })?;
            p.status = ProposalStatus::Accepted;
        } else {
            p.status = ProposalStatus::Rejected;
        }
        ws.store.put_proposal(&p)?;
        return Ok(Out::new(
            json!({"proposal": p}),
            format!("proposal {} {:?}", p.id, p.status).to_lowercase(),
        ));
    }
    let idx = ws.index()?;
    let from = resolve_story(
        &idx,
        story
            .as_deref()
            .ok_or_else(|| err!(Usage, "name the story to link from"))?,
    )?;
    let (kind, to) = match (supersedes, related) {
        (Some(t), None) => (LinkKind::Supersedes, t),
        (None, Some(t)) => (LinkKind::Related, t),
        _ => fail!(Usage, "give exactly one of --supersedes or --related"),
    };
    let to = resolve_story(&idx, &to)?;
    if !remove
        && kind == LinkKind::Supersedes
        && creates_cycle(&index::link_graph(&ws.store)?, &from, &to)
    {
        fail!(LinkCycle, "{from} supersedes {to} would create a cycle");
    }
    let entry = LinkEntry {
        op: if remove { LinkOp::Remove } else { LinkOp::Add },
        kind,
        from: from.clone(),
        to: to.clone(),
        at: id::now_ms(),
        author,
        proposal: None,
    };
    ws.store.append_link(&entry)?;
    Ok(Out::new(
        json!({"link": entry}),
        format!(
            "{} {from} {:?} {to}",
            if remove { "removed" } else { "linked" },
            kind
        )
        .to_lowercase(),
    ))
}

pub fn proposals(env: &Env) -> Result<Out> {
    let ws = env.ws()?;
    let open: Vec<LinkProposal> = ws
        .store
        .proposals()?
        .into_iter()
        .filter(|p| p.status == ProposalStatus::Open)
        .collect();
    let human = if open.is_empty() {
        "no open proposals".to_owned()
    } else {
        open.iter()
            .map(|p| {
                format!("{}  {} {:?} {}  ({})", p.id, p.from, p.kind, p.to, p.reason).to_lowercase()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    Ok(Out::new(json!({"proposals": open}), human))
}

// ---------------------------------------------------------------------------
// edit

pub struct EditArgs {
    pub rev: String,
    pub story: Option<String>,
    pub summary: Option<String>,
    pub add_decision: Vec<String>,
    pub add_unknown: Vec<String>,
    pub promote: Vec<usize>,
    pub demote: Vec<usize>,
    pub review: Option<String>,
}

pub fn edit(env: &Env, a: EditArgs) -> Result<Out> {
    let ws = env.ws()?;
    let idx = ws.index()?;
    let id = match &a.story {
        Some(s) => resolve_story(&idx, s)?,
        None if a.rev.starts_with("st_") => resolve_story(&idx, &a.rev)?,
        None => {
            let sha = ws.require_commit(&a.rev)?;
            let s = idx.for_commits(std::slice::from_ref(&sha))?;
            match s.as_slice() {
                [one] => one.story_id.clone(),
                [] => fail!(NotFound, "no landed story for {}", a.rev),
                _ => fail!(Ambiguous, "{} has several stories; pass --story", a.rev),
            }
        }
    };
    let mut s = ws
        .store
        .story(&id)?
        .ok_or_else(|| err!(NotFound, "no story {id}"))?;
    let author = ws.author();
    let ctx = Ctx::new(&ws, &s.work_unit);
    // Every edit is itself durable evidence, stated by the editor.
    let mut description = Vec::new();
    let mut new_claims: Vec<(String, &'static str)> = Vec::new();
    for d in &a.add_decision {
        new_claims.push((d.clone(), "decision"));
    }
    if let Some(t) = &a.summary {
        s.summary = crate::scrub::scrub(t).0;
        description.push(format!("summary set to: {t}"));
    }
    for u in &a.add_unknown {
        s.unknowns.push(crate::scrub::scrub(u).0);
        description.push(format!("unknown added: {u}"));
    }
    // Promote hypotheses (review only) into stated decisions.
    let mut promote = a.promote.clone();
    promote.sort_unstable_by(|x, y| y.cmp(x));
    let mut promoted = Vec::new();
    for i in promote {
        if i >= s.hypotheses.len() {
            fail!(Usage, "no hypothesis #{i}");
        }
        promoted.push(s.hypotheses.remove(i));
    }
    let mut demote = a.demote.clone();
    demote.sort_unstable_by(|x, y| y.cmp(x));
    for i in demote {
        if i >= s.decisions.len() {
            fail!(Usage, "no decision #{i}");
        }
        let c = s.decisions.remove(i);
        description.push(format!("demoted to hypothesis: {}", c.claim));
        s.hypotheses.push(Claim {
            kind: ClaimType::Inferred,
            status: None,
            ..c
        });
    }
    if let Some(r) = &a.review {
        if !matches!(r.as_str(), "reviewed" | "flagged" | "unreviewed") {
            fail!(Usage, "--review must be reviewed, flagged or unreviewed");
        }
        s.review.status = r.clone();
        description.push(format!("review: {r}"));
    }
    for p in &promoted {
        description.push(format!("promoted: {}", p.claim));
    }
    for (c, _) in &new_claims {
        description.push(format!("decision added: {c}"));
    }
    if description.is_empty() {
        fail!(Usage, "nothing to change; see `git story edit --help`");
    }
    let ev = ctx.event(
        EventType::Review,
        gitstory_proto::evidence::SourceType::HumanNote,
        &author,
        id::now_ms(),
        &format!("{author} edited story {id}"),
        Some(&description.join("\n")),
        format!("edit\0{id}\0{}\0{}", s.version, description.join("\n")).as_bytes(),
        None,
        Vec::new(),
    );
    ws.store.put_event(&ev)?;
    for p in promoted {
        s.decisions.push(Claim {
            kind: ClaimType::Stated,
            status: Some(ClaimStatus::Implemented),
            evidence: {
                let mut e = p.evidence;
                e.push(ev.id.clone());
                e
            },
            ..p
        });
    }
    for (c, _) in new_claims {
        s.decisions.push(Claim {
            claim: crate::scrub::scrub(&c).0,
            kind: ClaimType::Stated,
            status: Some(ClaimStatus::Implemented),
            evidence: vec![ev.id.clone()],
        });
    }
    s.version += 1;
    ws.store.put_story(&s)?;
    Ok(Out::new(
        json!({"story": story_view(env, &s), "event": ev.id}),
        format!(
            "{} is now v{}\n\n{}",
            s.story_id,
            s.version,
            render::markdown(&s)
        ),
    ))
}

// ---------------------------------------------------------------------------
// export / import

const EXPORT_SCHEMA: &str = "gitstory-export/v1";

pub fn export(env: &Env, format: &str, out: Option<PathBuf>) -> Result<Out> {
    let ws = env.ws()?;
    let dir = out.unwrap_or_else(|| env.cwd.join("gitstory-export"));
    fs::create_dir_all(&dir).context(|| format!("create {}", dir.display()))?;
    let mut objects = BTreeMap::new();
    for key in ws.store.all_keys()? {
        let data = ws.store.get_raw(&key)?.unwrap_or_default();
        let v: Value = if data.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&data).map_err(|e| err!(Internal, "corrupt {key}: {e}"))?
        };
        objects.insert(key, v);
    }
    let bundle = json!({
        "schema": EXPORT_SCHEMA,
        "repository": ws.store.repo,
        "exported_at": id::now_ms(),
        "objects": objects,
    });
    let json_path = dir.join("gitstory-export.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&bundle)?)?;
    let mut written = vec![json_path.clone()];
    if format == "markdown" {
        let idx = ws.index()?;
        let stories_dir = dir.join("stories");
        fs::create_dir_all(&stories_dir)?;
        let mut index = format!("# Stories for {}\n\n", ws.store.repo);
        for id in ws.store.story_ids()? {
            if let Some(s) = idx.story(&id)? {
                let p = stories_dir.join(format!("{id}.md"));
                fs::write(&p, render::markdown(&s))?;
                index.push_str(&format!(
                    "- [{}](stories/{id}.md) — `{}`{}\n",
                    s.summary,
                    &s.commit[..s.commit.len().min(10)],
                    if s.status == StoryStatus::Superseded {
                        " (superseded)"
                    } else {
                        ""
                    }
                ));
                written.push(p);
            }
        }
        let ip = dir.join("index.md");
        fs::write(&ip, index)?;
        written.push(ip);
    }
    Ok(Out::new(
        json!({"directory": dir, "files": written.len(), "objects": objects.len()}),
        format!("exported {} object(s) to {}", objects.len(), dir.display()),
    ))
}

pub fn import(env: &Env, src: PathBuf) -> Result<Out> {
    let ws = env.ws()?;
    let file = if src.is_dir() {
        src.join("gitstory-export.json")
    } else {
        src.clone()
    };
    let bundle: Value =
        serde_json::from_slice(&fs::read(&file).context(|| format!("read {}", file.display()))?)?;
    if bundle["schema"] != EXPORT_SCHEMA {
        fail!(Usage, "{} is not a GitStory export", file.display());
    }
    let objects = bundle["objects"].as_object().cloned().unwrap_or_default();
    let (mut written, mut same) = (0, 0);
    // Check everything first: an import never half-applies.
    for (key, v) in &objects {
        if key.contains("..") || key.starts_with('/') {
            fail!(Usage, "export contains an unsafe key {key:?}");
        }
        if let Some(existing) = ws.store.get_raw(key)? {
            let existing: Value = if existing.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&existing).unwrap_or(Value::Null)
            };
            if &existing != v && !key.starts_with("proposals/") {
                fail!(StoreConflict, "{key} already exists with different content");
            }
        }
    }
    for (key, v) in &objects {
        let data = if v.is_null() {
            Vec::new()
        } else {
            let mut d = serde_json::to_vec_pretty(v)?;
            d.push(b'\n');
            d
        };
        if ws.store.get_raw(key)?.as_deref() == Some(&data[..]) {
            same += 1;
            continue;
        }
        ws.store.put_raw(key, &data)?;
        written += 1;
    }
    Ok(Out::new(
        json!({"written": written, "unchanged": same, "repository": ws.store.repo}),
        format!("imported {written} object(s) ({same} already present)"),
    ))
}

// ---------------------------------------------------------------------------
// status, gc

pub fn status(env: &Env) -> Result<Out> {
    let ws = env.ws()?;
    let now = id::now_ms();
    let expiry = ws.cfg.draft_expiry_ms();
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    for wu in WorkUnit::all(&ws)? {
        let drafts = wu.drafts()?.len();
        let events = wu.events()?.len();
        let expires_at = wu.meta.created_at + expiry;
        if wu.meta.landed.is_empty() && now > expires_at - 7 * 86_400_000 {
            warnings.push(format!(
                "work unit {} has not landed; its drafts expire {}",
                wu.meta.work_unit,
                if now >= expires_at {
                    "now (run `git story gc`)".to_owned()
                } else {
                    format!("in {} day(s)", (expires_at - now) / 86_400_000 + 1)
                }
            ));
        }
        units.push(json!({"work_unit": wu.meta.work_unit, "events": events, "drafts": drafts, "landed": wu.meta.landed, "created_at": wu.meta.created_at, "expires_at": expires_at}));
    }
    let (endpoint, local) = model::endpoint(&ws.cfg.model);
    let trusted = local || config::trusted(&endpoint)?;
    if !trusted {
        warnings.push(format!(
            "model endpoint {endpoint} is not trusted; run `git story trust`"
        ));
    }
    let stories = ws.store.story_ids()?.len();
    let open_proposals = ws
        .store
        .proposals()?
        .iter()
        .filter(|p| p.status == ProposalStatus::Open)
        .count();
    let mut human = vec![
        format!(
            "repository {} · store {}",
            ws.store.repo,
            ws.store.backend.describe()
        ),
        format!(
            "{stories} landed stor{} · {open_proposals} open link proposal(s)",
            if stories == 1 { "y" } else { "ies" }
        ),
        format!(
            "model {} {} ({})",
            ws.cfg.model.provider,
            ws.cfg.model.name,
            if local { "local" } else { "external" }
        ),
        format!(
            "egress: inference {}, events {}, diff {}, excerpts {}",
            ws.cfg.egress.inference,
            ws.cfg.egress.send_events,
            ws.cfg.egress.send_diff,
            ws.cfg.egress.send_excerpts
        ),
    ];
    for u in &units {
        human.push(format!(
            "work unit {}: {} event(s), {} draft(s)",
            u["work_unit"].as_str().unwrap_or(""),
            u["events"],
            u["drafts"]
        ));
    }
    for w in &warnings {
        human.push(format!("warning: {w}"));
    }
    Ok(Out::new(
        json!({"repository": ws.store.repo, "store": ws.store.backend.describe(), "stories": stories, "open_proposals": open_proposals, "work_units": units, "model": {"endpoint": endpoint, "local": local, "trusted": trusted}, "egress": ws.cfg.egress, "warnings": warnings}),
        human.join("\n"),
    ))
}

pub fn gc(env: &Env, dry_run: bool) -> Result<Out> {
    let ws = env.ws()?;
    let now = id::now_ms();
    let expiry = ws.cfg.draft_expiry_ms();
    let mut removed = Vec::new();
    for wu in WorkUnit::all(&ws)? {
        // Landed: the store keeps what the story cites; the rest goes.
        // Never landed: drafts expire.
        let done = wu.meta.landed_at.is_some_and(|t| now - t > 86_400_000);
        let expired = wu.meta.landed.is_empty() && now >= wu.meta.created_at + expiry;
        if done || expired {
            removed.push(wu.meta.work_unit.clone());
            if !dry_run {
                wu.remove()?;
            }
        }
    }
    Ok(Out::new(
        json!({"dry_run": dry_run, "removed": removed}),
        if removed.is_empty() {
            "nothing to collect".to_owned()
        } else {
            format!(
                "{} {}",
                if dry_run { "would remove" } else { "removed" },
                removed.join(", ")
            )
        },
    ))
}

// ---------------------------------------------------------------------------
// hooks, skill

pub fn hooks(env: &Env, action: &str, h: hooks::Harness, scope: hooks::Scope) -> Result<Out> {
    let repo = Repo::discover(&env.cwd).ok();
    let (path, changed, installed) = hooks::manage(h, scope, repo.as_ref(), action)?;
    Ok(Out::new(
        json!({"path": path, "changed": changed, "installed": installed, "command": hooks::DRAFT_COMMAND}),
        format!(
            "{}: {}",
            path.display(),
            if installed {
                "installed"
            } else {
                "not installed"
            }
        ),
    ))
}

pub fn skill(env: &Env, action: &str, scope: hooks::Scope) -> Result<Out> {
    if action == "show" {
        return Ok(Out::new(json!({"skill": hooks::SKILL}), hooks::SKILL));
    }
    let repo = Repo::discover(&env.cwd).ok();
    let (path, changed) = hooks::install_skill(scope, repo.as_ref().map(|r| r.root.as_path()))?;
    Ok(Out::new(
        json!({"path": path, "changed": changed}),
        format!(
            "{} {}",
            if changed { "wrote" } else { "unchanged:" },
            path.display()
        ),
    ))
}
