# GitStory PRD

**Version:** 0.2 draft
**Date:** 15 October 2026
**Owner:** Ran Aroussi
**Source:** ghf concept v0.4, ghf PRD v0.2, GitStory PRD v0.1 review
**Related:** ghf PRD (independent; optional evidence source)

## 1. Summary

GitStory keeps the "why" behind landed code. It collects evidence from agent sessions, tests, reviews and Git while work happens. It distills that evidence into a short record of decisions, checks every claim against the code that actually landed, and links the result to the landed commit. Future agents and humans can ask why code looks the way it does, for a commit, a file or a line.

GitStory stores **distilled decisions, not raw transcripts**. Every authoritative claim carries its own small proof, so it stays auditable after the original sources are gone. When the evidence doesn't say why, the story says so. Decisions can supersede older decisions, so memory stays current instead of piling up contradictions.

GitStory writes nothing into Git. It works without ghf. With ghf installed, it also sees the attempts made between commits.

Tagline: **Evidence-backed engineering memory for Git commits.**

Positioning: **Entire remembers what an agent said. GitStory remembers which engineering decisions were actually justified.**

## 2. Problem

A commit records what changed. A good message adds a line or two of why. Everything else is lost: the approaches that failed, the constraint that forced a choice, the test that proved it, the thing we decided not to do.

With agents, this loss gets worse and more expensive:

1. **Agents repeat rejected ideas.** A new agent sees the Redis lock is gone and adds it back, because nothing told it why it was removed.
2. **Humans can't review intent.** A reviewer sees a 400-line diff from a pipeline and has to guess the reasoning.
3. **Transcripts don't fix it.** They are huge, noisy and mostly dead ends. They hold secrets, customer data and model chatter. Nobody reads them, and agents can't use them without burning their context.
4. **Memory goes stale.** Even good records contradict each other over time. A decision made in March may be reversed in June and reconsidered in October. Without links between them, an agent can't tell which one holds now.

The useful part of a session is small: the goal, the decisions, the alternatives tried and dropped, and the proof. GitStory keeps that part, keeps it current, and drops the rest.

## 3. Who it's for

**Customer zero: Cloop.** Every Cloop task already produces a PRD, a plan, coding sessions, test runs, review rounds and a PR. Most of it is thrown away after merge.

**Primary reader: future agents.** Before an agent changes code, it asks GitStory why that code exists. This is the main design target. Output must be compact, structured and machine-readable.

**Secondary reader: humans.** Reviewers during PR review, engineers doing archaeology on a line, people taking an escalation.

**First external users:** teams running agent pipelines who want their agents to stop re-learning old decisions. This may become the broader product of the two. ghf helps agents recover work. GitStory helps them avoid repeating mistakes, which almost every AI-assisted team needs.

## 4. Core use cases

**S1. Ask before changing.** An agent is about to edit `lock.py`. It runs `git story blame lock.py:40-80 --brief` and learns that a distributed lock was removed because it caused deadlocks under retry. It doesn't add it back, or if it must, it records why the constraint changed.

**S2. Review with intent.** A human reviewing a Cloop PR reads the story next to the diff: goal, decisions, rejected options, test evidence, open unknowns.

**S3. Search the past.** "Why did we drop Redis locking?" returns the current decision on locking, and the older ones it replaced.

**S4. Take an escalation.** When Cloop's review loop escalates, the human reads a draft of what the agents decided and tried so far.

**S5. Follow a decision through time.** Locking was added in A, removed in B, and brought back under new constraints in C. The agent sees C as current, with B and A linked as history.

**S6. Leave with your memory.** A team exports all stories and their evidence as Markdown or JSON and imports them elsewhere, without any change to Git.

## 5. Why GitStory and not the alternatives

| Option | What it does well | Where it falls short |
|---|---|---|
| Good commit messages | Plain Git, everywhere | Short by habit; rejected options and evidence rarely fit; agents write generic ones |
| PR descriptions | Visible in review | Tied to one host; not linked to lines; often written before the work ends |
| ADRs | Deliberate, durable, can be superseded | Manual, for big decisions only; most code-level decisions never get one |
| Entire | Captures full sessions, links them to commits, search, resume | Stores raw transcripts; adds an `Entire-Checkpoint` trailer to commit messages; data lives in the repo |
| git-story.dev | Turns repo history into narrative content | Different job: storytelling about history, not memory for agents |
| GitStory | Cited decisions per landed commit, checked against landed code, linked over time, built for agents to query, nothing in Git, portable | New tool; quality depends on evidence and synthesis |

## 6. Goals and non-goals

### Goals

- Produce a cited story for every landed commit that has agent evidence.
- Describe only decisions the landed code actually reflects. Keep discarded work as history, clearly marked.
- Never present a guess as a fact, and never let a guess become a constraint without review.
- Keep every authoritative claim auditable after its original source is gone.
- Never send raw transcripts anywhere by default. Make every outbound byte a configured, logged choice.
- Let agents query by commit, file, line and free text, and see which decisions are current.
- Keep stories portable: full export and import, no lock-in.
- Write nothing into Git. Work without ghf.

### Non-goals (MVP)

- Storing or replaying full transcripts.
- Automatic detection that one decision supersedes another.
- Code review, quality scoring, or AI-versus-human attribution.
- Automatic mapping through every rebase and cherry-pick.
- Stories for commits with no agent evidence (human-only history). P2 at most.
- Replacing ADRs or PR descriptions.

## 7. Concepts

| Term | Meaning | Lifetime |
|---|---|---|
| Source | Where evidence comes from: agent session, test run, review, ticket, Git, ghf trace, human note | Owned by the source; may disappear |
| Work unit | The task that evidence is collected for (a Cloop ticket, a branch, a session) | Until its stories land |
| Evidence event | A small extracted fact with a durable part and an optional external pointer | Kept with the story if cited |
| Story | The distilled, reconciled record of decisions for one landed commit | Durable |
| Claim | One statement in a story, with a type, a status and citations | Part of a story |
| Association | Link from a story to repository ID plus landed commit SHA | Durable |
| Story link | Explicit relationship between stories: supersedes, related | Durable |

### 7.1 Evidence events: durable and external

Every evidence event has two parts.

**Durable part (required).** The smallest self-contained proof, kept with the story forever:

- a typed fact (for example `test: pytest tests/retry -k lock, 50/50 pass`);
- a short scrubbed excerpt where the fact is a statement (for example the reason an agent gave), capped in length;
- a content hash of the source span, so the excerpt can be verified if the source still exists;
- source type, time and author.

**External part (optional).** A pointer to the original: session file and offset, CI run URL, ticket ID, PR comment. It may stop working. Nothing depends on it.

**Rule:** every authoritative claim must be supported by the durable parts of its citations alone. The validation pass checks only durable parts, so a claim that passes today still passes after every source is gone.

Event types: `goal`, `decision`, `reason`, `attempt`, `abandon`, `test`, `constraint`, `review`.

### 7.2 Claim types

- **Observed:** proven by the evidence itself (a diff, a test result, a revert).
- **Stated:** someone said it explicitly (an agent, the ticket, a reviewer, a human editing the story).
- **Inferred:** the synthesizer's reading. Allowed and labeled, but **never authoritative**:
  - inferred claims live in a separate `hypotheses` section, never in `decisions`;
  - they are excluded from `--brief` output by default;
  - the agent skill tells agents not to treat them as constraints;
  - only a reviewer can promote one, which turns it into a `stated` claim with the reviewer as source.

A claim with no citation is invalid and is dropped.

### 7.3 Claim status after landing

Reconciliation against the landed commit gives each claim a status:

- **Implemented:** the landed diff reflects it. Only implemented claims go in `decisions`.
- **Rejected:** it was tried and dropped, with evidence of the attempt or the reason. Goes in `rejected`, as history.
- **Discarded:** it appeared in a draft but the landed code doesn't reflect it, and there's no evidence of why. Goes in `discarded`, hidden from `--brief`, never presented as a decision.

### 7.4 Story shape

Structured data first, readable text second.

```json
{
  "story_id": "st_8f2c",
  "schema": "gitstory/v1",
  "repo": "org/project",
  "commit": "a83f2c1",
  "work_unit": "cloop:TCK-482",
  "status": "current",
  "supersedes": ["st_5b10"],
  "related": ["st_3a77"],
  "summary": "Replaced Redis locking with optimistic row versioning to remove retry deadlocks.",
  "goal": { "claim": "Stop duplicate job execution under retries", "type": "stated", "evidence": ["ev_02"] },
  "decisions": [
    { "claim": "Replaced the Redis lock with optimistic row versioning", "type": "observed", "status": "implemented", "evidence": ["ev_12", "ev_19"] },
    { "claim": "The Redis lock caused deadlocks when jobs retried", "type": "stated", "status": "implemented", "evidence": ["ev_15"] }
  ],
  "rejected": [
    { "claim": "Lease timeout on the lock; tests still flaked", "type": "observed", "status": "rejected", "evidence": ["ev_17", "ev_18"] }
  ],
  "discarded": [],
  "evidence_summary": [
    { "claim": "Retry suite passed 50 of 50 runs", "type": "observed", "evidence": ["ev_21"] }
  ],
  "hypotheses": [
    { "claim": "Row versioning was likely chosen to avoid a new dependency", "type": "inferred", "evidence": ["ev_12"] }
  ],
  "limitations": [],
  "unknowns": ["Why the 5s retry window was chosen is not in the evidence"],
  "paths": ["src/jobs/lock.py", "src/jobs/retry.py"],
  "links": { "pr": "...", "ticket": "..." },
  "review": { "status": "unreviewed" },
  "egress": "egr_91c4"
}
```

No fixed word limit. The default Markdown view shows the summary, decisions and unknowns, with rejected options, evidence and hypotheses as expandable sections. `--brief` gives agents a compact fixed shape.

### 7.5 Story relationships

- **supersedes:** this story's decisions replace those of the listed stories, for the same concern.
- **related:** useful context, no replacement.
- **status:** `current`, or `superseded` when another story supersedes it. Derived from links, never guessed.

Links are explicit. They are created by:

1. the agent, through `git story note --supersedes <story>` when it knowingly changes a past decision (the skill asks for this after a `blame` lookup);
2. a human or reviewer, through `git story link`;
3. landing reconciliation, which **proposes** candidates (stories found by blaming the lines the landed commit changed) for a human or reviewer agent to accept. Proposals are never applied on their own.

### 7.6 Lifecycle

```text
collecting (work unit)  ->  draft  ->  reconciled + landed  ->  reviewed (optional)
                                                |
                                     current / superseded
```

Synthesis is **event-driven**. Evidence is collected per work unit. Drafts are made at meaningful events, and the story is associated with landed commits:

- **Pipeline events (Cloop):** task done, review rejects an approach, PR merged.
- **Plain Git users:** a post-commit hook, or `git story draft`.

One work unit can produce several landed commits. Each landed commit gets its own story, sharing the work unit ID.

## 8. Invariants

1. GitStory writes nothing into Git: no commits, trailers, notes, refs or objects. Local client state lives under the Git dir (for example `.git/story/`).
2. The normal commit message stays useful on its own. GitStory never requires a pointer in it.
3. Every claim cites at least one evidence event. Uncited claims don't ship.
4. **Every authoritative claim is supported by durable evidence alone.** No authoritative claim depends on a source that may disappear.
5. **Inferred claims are never authoritative.** They live apart from decisions and need review to be promoted.
6. **A landed story describes only what the landed code reflects.** Discarded and rejected work is history, never presented as an implemented decision.
7. When the evidence doesn't explain a decision, the story lists it under unknowns.
8. Raw transcripts never leave the local machine by default. Anything sent to an external model is set by policy and logged.
9. Story relationships are explicit. GitStory never decides on its own that one decision replaced another.
10. Stories are exportable in full, with their durable evidence, at any time.
11. GitStory works without ghf. ghf is an optional source.
12. A story is never attached to a commit by diff similarity alone.

## 9. Functional requirements

Priority: **P0** = MVP, **P1** = soon after, **P2** = later.

### 9.1 Setup

- **P0.** `git story init` writes a tracked `.gitstory` file with store location, collection policy and egress policy. No credentials in it.
- **P0.** First contact with a remote store or model endpoint asks for trust. Trust and credentials live outside the repo.

### 9.2 Evidence adapters

- **P0.** Claude Code session adapter. Reads the local session log, extracts events with durable parts, scrubs them, never uploads the raw log.
- **P0.** Git adapter: commit metadata, diffs, changed paths, reverts.
- **P0.** Test adapter: test commands and results from the session, plus `git story note --test "<cmd>" --result pass|fail`.
- **P0.** Notes: `git story note "<text>" [--type decision|reason|constraint] [--supersedes <story>]`.
- **P0.** Cloop adapter: ticket, PRD, approved plan, review findings and resolutions, and lifecycle events (task done, review rejection, PR merged).
- **P1.** Second harness adapter.
- **P1.** ghf adapter: reads traces for attempts and abandons. Places a ghf hold while reading, releases it after.
- **P2.** Cursor Agent Trace import for attribution context.

### 9.3 Privacy and egress

Local distillation is not the same as local inference. GitStory separates the two.

**Processing that always runs locally:** reading sources, extracting events, scrubbing, building the evidence bundle, building the search index.

**Synthesis and validation** may run on a local or an external model. The egress policy in `.gitstory` decides what an external model may receive:

| Setting | Values | Default |
|---|---|---|
| `inference` | `local` (local model only) or `external` | `external` |
| `send.events` | durable parts of scrubbed events | on |
| `send.diff` | landed and draft diff hunks (this is source code) | on, can be turned off |
| `send.excerpts` | statement excerpts inside events | on |
| `send.paths_ignored` | anything under `.storyignore` | never, not configurable |
| `send.transcripts` | raw session logs | never by default; requires explicit opt-in |
| `send.secrets` | anything the scrubber flags | never, not configurable |

- **P0.** The policy above, enforced in the client before any request leaves the machine.
- **P0.** **Egress manifest.** For each story, a record of which model endpoint was called and which event IDs and file hunks were sent. Stored with the story and exportable.
- **P0.** Secret scrubbing on every excerpt: common key formats, `.env` values, tokens, high-entropy strings.
- **P0.** Excerpt length cap per event.
- **P0.** `.storyignore`: events touching these paths keep only the path, never content, and nothing about them is sent.
- **P0.** `excerpts = none` mode keeps only typed facts. In this mode, stated claims that rely on an excerpt cannot meet the durable evidence rule, so they are downgraded to unknowns. The docs say so.
- **P1.** PII scrubbing.
- **P1.** Story-level access control in the shared store.

### 9.4 Synthesis

- **P0.** Runs from the evidence bundle, through the configured model, inside the egress policy.
- **P0.** Output must match `gitstory/v1`. Invalid output is retried, then fails with a stable error code.
- **P0.** Inferred claims go only to `hypotheses`.
- **P0.** If no stated or observed reason exists for the main change, the story says so under unknowns.
- **P1.** Regenerate from retained durable evidence with a newer model, keeping the old version.

### 9.5 Validation

- **P0.** A separate pass walks every claim and checks that the **durable parts** of its citations support it. External pointers are not consulted.
- **P0.** Test claims are checked against the typed test fact, not against the model's reading.
- **P0.** Unsupported claims are dropped, or moved to `hypotheses` if they remain plausible.

### 9.6 Landing and reconciliation

- **P0.** `git story land --work-unit <id> --to <landed-sha>` (or `--from <draft-sha>...`) starts reconciliation. Cloop calls it on PR merge.
- **P0.** GitStory checks that the landed commit exists and is reachable from the main branch. It does not guess.
- **P0.** **Reconciliation pass:**
  1. Gather all drafts and evidence for the work unit.
  2. Compare every claim with the landed diff.
  3. Mark each claim `implemented`, `rejected` or `discarded` (section 7.3).
  4. Keep rejected work as history and discarded work as hidden history.
  5. Produce the landed story with citations that support the final code, then run validation again.
  6. Blame the lines the landed commit changed and propose `supersedes` or `related` links to existing stories.
- **P0.** If no claims survive as implemented, the landed story says so and lists unknowns. It is still created.
- **P0.** Drafts whose work unit never lands expire after a set period (default 30 days), with a warning first.
- **P1.** Auto-land from Git host webhooks when the PR merges.
- **P1.** `git story relink` for cherry-picks and history rewrites, always explicit.

### 9.7 Relationships

- **P0.** `git story link <story> --supersedes <story>` and `--related <story>`.
- **P0.** `git story link --accept <proposal>` and `--reject <proposal>` for reconciliation proposals.
- **P0.** A story's `status` becomes `superseded` when any accepted link supersedes it. Links are append-only. Removing one is a new recorded action.
- **P0.** Cycles are refused.

### 9.8 Retrieval

- **P0.** `git story [<rev>]`: the story for a commit, default `HEAD`.
- **P0.** `git story blame <path>[:<lines>]`: runs `git blame`, collects the commits behind those lines, and returns their stories. **It then follows supersession forward.** If a returned story is superseded, the current story that replaced it is returned too, marked as current. The agent sees the decision that holds now, even if blame alone would show an older one.
- **P0.** `git story path <path>`: every story that ever touched the path, including ones whose lines are gone, with current decisions first. Decisions can outlive their lines, so this complements blame.
- **P0.** `git story history <story>`: the supersession chain for a decision, oldest to newest.
- **P0.** `git story search "<query>"`: full-text search over summaries, claims, goals and paths. Results show status.
- **P1.** Semantic search with embeddings.
- **P0.** `--brief`: summary, implemented decisions, rejected options and unknowns, current stories only. No hypotheses, no discarded work.
- **P0.** Every command supports `--json` with a versioned schema, stable error codes and exit codes, the same contract as ghf.

### 9.9 Review

- **P0.** `git story edit <rev>` with versioned history. Human-added claims are `stated` with the editor as source and a durable event created from the edit.
- **P0.** Review can promote a hypothesis, demote a claim, and accept or reject link proposals.
- **P1.** Post the Markdown story as a PR comment. This lives on the Git host, not in Git history.
- **P1.** Reviewer agent can mark a story reviewed or flag claims.

### 9.10 Storage

- **P0.** One store format: story JSON, the durable parts of cited events, links, egress manifests, and an index. Keyed by repository ID and commit SHA.
- **P0.** Two backends that share that format: a **local directory**, and an **S3-compatible bucket** (self-hostable, for example MinIO) for teams and fleets.
- **P0.** Local search index built from the store, rebuildable at any time.
- **P0.** `git story export --format markdown|json` and `git story import`. Export includes durable evidence, links and egress manifests.
- **P1.** GitStory service with auth, access control and server-side search.
- **P2.** Hosted service.

Retention: stories, links, cited durable evidence and egress manifests are durable. Uncited evidence is dropped after landing. A repo or org policy can set a maximum age.

### 9.11 Agent skill

- **P0.** An installable skill that teaches agents to:
  - run `git story blame --brief` before changing existing code, and `git story path` for files whose history matters;
  - treat implemented decisions and rejected options as strong signals;
  - treat hypotheses as unverified, never as constraints;
  - when knowingly changing a past decision, record why with `git story note --supersedes <story>`;
  - add `git story note` lines when they make a real decision or abandon an approach;
  - never edit an old story to fit a new change.

## 10. Illustrative CLI

Not final.

```bash
git story init
git story note "<text>" [--type decision|reason|constraint] [--supersedes <story>]
git story note --test "<cmd>" --result pass|fail

git story draft [--work-unit <id>]
git story land --work-unit <id> --to <sha>

git story [<rev>] [--brief] [--json]
git story blame <path>[:<lines>] [--brief]
git story path <path>
git story history <story>
git story search "<query>"

git story link <story> --supersedes <story> | --related <story>
git story link --accept <proposal> | --reject <proposal>
git story edit <rev>

git story export --format markdown|json [--out <dir>]
git story import <dir>
```

## 11. Non-functional requirements

Targets are proposals. M0 replaces them with real numbers.

| Area | Proposed target |
|---|---|
| Draft time after an event | p95 under 60 s, off the critical path |
| Reconciliation at land | p95 under 2 min, off the critical path |
| Retrieval (`blame --brief`, `path`) | p95 under 500 ms from local index |
| `--brief` size | Under 2 KB JSON per story |
| Synthesis cost | Report per story; target set in M0 |
| Uncited claims shipped | Zero |
| Authoritative claims without durable support | Zero |
| Raw transcripts sent by default | Zero |
| Platforms | Linux and macOS at launch |

Commits and merges never wait on GitStory. A failed draft or reconciliation logs a warning and can be retried.

## 12. Success metrics

Measured in Cloop over the pilot.

**Usefulness**

- Share of agent edits to existing code preceded by a `git story` lookup.
- **Repeat-mistake catches:** an agent was about to bring back a rejected option and changed course after reading a story.
- **Contradictions surfaced:** an agent or human found a stale decision through `history` or a superseded marker.
- Human rating of accuracy and usefulness on a weekly sample (1 to 5).
- Reviewer time per PR, with and without a story, where measurable.

**Trust**

- Audited claim accuracy on a weekly sample, checked against durable evidence only. Target 95% or higher.
- Share of landed stories with claims marked `discarded`. Shows how much reconciliation protects us.
- Share of stories with at least one unknown. Zero would be suspicious.
- Link proposals accepted versus rejected.

**Cost and coverage**

- Synthesis and reconciliation cost per story.
- Share of landed commits with a landed story.
- Store size per story, including durable evidence.

**Kill rule.** If agents don't use stories, or audited accuracy stays below target after tuning, we stop or rethink before building the service.

## 13. Milestones

**M0. Prove the story (1 to 2 weeks).** No product code. Take 20 recent Cloop tasks. Build evidence bundles by script, with durable parts. Generate drafts, reconcile against the merged commit, validate. Humans rate them. Pick two or three tasks that touched the same code over time and link them by hand, to test supersession. Then run new Cloop tasks on that code with and without the stories in context, and see if behavior changes. Measure egress size and synthesis cost.

**M1. Local CLI.** `git-story` with init, notes, Claude Code and Git adapters, test events, durable evidence, scrubbing, egress policy and manifest, synthesis, validation, draft, show, local store, export. JSON contract v1.

**M2. Landing and memory over time.** Work units, `land` with reconciliation, link proposals, `link`, `history`, supersession-aware `blame`, `path`, `search`, `--brief`, S3-compatible store.

**M3. Cloop integration.** Cloop adapter with lifecycle events, agent skill, PR comment, ghf adapter with holds.

**M4. Cloop pilot.** All Cloop tasks produce stories for a month. Review section 12.

## 14. Acceptance scenarios

1. A plain Git repo with no ghf. Run `git story init`.
2. An agent works a task in Claude Code, tries one approach, drops it, and lands another. It records one `git story note` reason.
3. A draft appears after the task event. The commit was not delayed.
4. Plant an API key in the session. It appears nowhere in the store or in any egress manifest.
5. The egress manifest lists exactly what was sent to the model. No raw transcript was sent.
6. Set `inference = local`. No request leaves the machine.
7. Two drafts exist for the work unit. One describes an approach removed during review. After squash-merge and `git story land`, that approach is in `rejected` or `discarded`, never in `decisions`.
8. Delete the original session log and break the CI link. Validation re-run on the landed story still passes for every authoritative claim.
9. Commit A adds a lock, commit B removes it, commit C reintroduces it with `--supersedes` on B's story. `git story blame` on C's lines returns C as current. `git story blame` on a line last touched by B returns B and C, with C marked current. `git story history` shows A, B, C.
10. Reconciliation proposes a link. It is not applied until accepted.
11. `git story blame --brief` returns no hypotheses and no discarded claims.
12. On a second machine with the shared store, `blame` returns in under 500 ms.
13. Export and import into an empty store. Stories, links, durable evidence and egress manifests are identical.
14. With ghf installed, the story also cites trace attempts. GitStory placed and released a ghf hold. Prune the trace. The story still validates.
15. Git history, refs and commit messages are identical to a run without GitStory.

## 15. Risks

- **Stories that sound right but aren't.** Still the biggest risk. Defenses: citations, durable evidence, the validation pass, reconciliation, hypotheses kept apart, weekly audits.
- **Missing links.** Supersession depends on agents and humans recording it. Unlinked contradictions will exist. Reconciliation proposals and the skill's `--supersedes` habit reduce this. Automatic detection stays out of scope until we have data.
- **Thin evidence.** Many sessions never state a reason. Stories will have many unknowns. `git story note` and the Cloop adapter are the main fixes.
- **Code sent to external models.** Diffs are source code. Some teams will need `send.diff = off` or local inference, which lowers quality. Be clear about the trade.
- **Agents ignore it.** M0 tests this before we build much.
- **Harness log formats change.** Keep adapters small, versioned and tested against real logs.
- **Cost at fleet scale.** Reconciliation adds a second model pass per landed commit. Measure in M0.
- **Entire moves toward distillation.** Our remaining difference is landed-code reconciliation, durable evidence, supersession, privacy and nothing in Git. Watch them.
- **Name.** git-story.dev exists. Check trademark and CLI name before launch.

## 16. Decisions made in v0.2

| Area | Decision |
|---|---|
| Story relationships | Explicit `supersedes` and `related` links; status derived from links; no automatic supersession |
| Retrieval | `blame` follows supersession forward; `path` covers decisions whose lines are gone; `history` shows the chain |
| Landing | Reconciliation against the landed diff; claims marked implemented, rejected or discarded |
| Evidence | Durable part required, external pointer optional; validation uses durable parts only |
| Inferred claims | Separate `hypotheses` section; never authoritative; excluded from `--brief`; promoted only by review |
| Privacy | Local processing always; inference local or external by policy; egress manifest per story |
| Length | No fixed word limit; concise default view with expandable sections; compact `--brief` |
| Synthesis timing | Event-driven per work unit; associated with landed commits |

## 17. Open questions

1. Default model for synthesis, and should validation use a different one?
2. Excerpt cap per event: how short can proof be and still convince an auditor?
3. Should reviewed stories become immutable, with corrections as new versions?
4. Default draft expiry and maximum story age.
5. Do we backfill stories for historical Cloop work in M4, where associations are reliable?
6. Should `supersedes` work at claim level rather than story level? Story level is simpler; claim level is more precise when one commit changes several decisions.
7. Commercial model. Proposal: open-source CLI, format and adapters; paid hosted store and search later.
8. Final name and CLI grammar.

## 18. Relationship to ghf

GitStory is independent of ghf. When ghf is present, GitStory reads traces as an extra source of attempts and abandons. It places a ghf hold while reading and releases it after. Durable evidence from the trace is kept with the story, so pruning ghf never breaks a story.
