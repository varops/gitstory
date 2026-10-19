//! Story views: Markdown for people, `--brief` for agents (PRD §7.4, §9.8).

use gitstory_proto::story::{Claim, Stage, Story, StoryStatus};
use serde_json::{Value, json};

/// The compact agent view: summary, implemented decisions, rejected options
/// and unknowns. No hypotheses, no discarded claims.
pub fn brief(s: &Story) -> Value {
    let claims = |l: &[Claim]| -> Vec<Value> {
        l.iter()
            .map(|c| json!({"claim": c.claim, "type": c.kind}))
            .collect()
    };
    json!({
        "story_id": s.story_id,
        "commit": s.commit,
        "status": s.status,
        "stage": s.stage,
        "summary": s.summary,
        "decisions": claims(&s.decisions),
        "rejected": claims(&s.rejected),
        "unknowns": s.unknowns,
        "supersedes": s.supersedes,
    })
}

fn bullet(c: &Claim) -> String {
    let kind = format!("{:?}", c.kind).to_lowercase();
    format!("- {} _({kind}; {})_\n", c.claim, c.evidence.join(", "))
}

pub fn markdown(s: &Story) -> String {
    let mut m = String::new();
    let short = &s.commit[..s.commit.len().min(12)];
    m.push_str(&format!("# {}\n\n", s.summary));
    let mut tags = vec![
        format!("commit `{short}`"),
        format!("work unit `{}`", s.work_unit),
    ];
    if s.stage == Stage::Draft {
        tags.push("**draft**".into());
    }
    if s.status == StoryStatus::Superseded {
        tags.push("**superseded**".into());
    }
    tags.push(format!("story `{}` v{}", s.story_id, s.version));
    m.push_str(&format!("{}\n\n", tags.join(" · ")));
    m.push_str(&format!("**Goal:** {}\n\n", s.goal.claim));
    if !s.decisions.is_empty() {
        m.push_str("## Decisions\n\n");
        s.decisions.iter().for_each(|c| m.push_str(&bullet(c)));
        m.push('\n');
    }
    if !s.unknowns.is_empty() {
        m.push_str("## Unknowns\n\n");
        s.unknowns
            .iter()
            .for_each(|u| m.push_str(&format!("- {u}\n")));
        m.push('\n');
    }
    let section = |m: &mut String, title: &str, list: &[Claim]| {
        if !list.is_empty() {
            m.push_str(&format!(
                "<details><summary>{title} ({})</summary>\n\n",
                list.len()
            ));
            list.iter().for_each(|c| m.push_str(&bullet(c)));
            m.push_str("\n</details>\n\n");
        }
    };
    section(&mut m, "Rejected options", &s.rejected);
    section(&mut m, "Evidence", &s.evidence_summary);
    section(&mut m, "Hypotheses (unverified)", &s.hypotheses);
    section(&mut m, "Discarded during the work", &s.discarded);
    if !s.limitations.is_empty() {
        m.push_str("## Limitations\n\n");
        s.limitations
            .iter()
            .for_each(|l| m.push_str(&format!("- {l}\n")));
        m.push('\n');
    }
    if !s.supersedes.is_empty() || !s.related.is_empty() {
        m.push_str("## Links\n\n");
        s.supersedes
            .iter()
            .for_each(|x| m.push_str(&format!("- supersedes `{x}`\n")));
        s.related
            .iter()
            .for_each(|x| m.push_str(&format!("- related `{x}`\n")));
        m.push('\n');
    }
    if !s.paths.is_empty() {
        m.push_str(&format!(
            "Paths: {}\n",
            s.paths
                .iter()
                .map(|p| format!("`{p}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    m
}

/// One line per story for lists.
pub fn line(s: &Story) -> String {
    let flag = match (s.stage, s.status) {
        (Stage::Draft, _) => " [draft]",
        (_, StoryStatus::Superseded) => " [superseded]",
        _ => "",
    };
    format!(
        "{}  {}  {}{}",
        s.story_id,
        &s.commit[..s.commit.len().min(10)],
        s.summary,
        flag
    )
}
