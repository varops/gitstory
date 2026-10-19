# ghf PRD

**Version:** 0.2 draft
**Date:** 15 October 2026
**Owner:** Ran Aroussi
**Source:** ghf concept v0.4, PRD v0.1 review
**Related:** GitStory PRD (separate, follows this one)

## 1. Summary

ghf (Git High Frequency) is a versioned working history for AI coding agents. It saves the state of a worktree every time an agent finishes an edit. These saves are called checkpoints. Checkpoints form traces, one per unit of work. Any agent on any authorized machine can list, diff, restore, apply or fork a trace.

ghf sits next to Git. It never writes into commits, messages, refs, notes or the object database. Accepted work still lands as ordinary Git commits and pull requests. Plain Git users don't need to know ghf exists.

Tagline: **High-frequency source control for AI agents. Git-compatible by design.**

- **Git** holds accepted history.
- **ghf** holds recoverable working history.
- **GitStory** holds durable engineering memory (separate product).

## 2. Problem

Agents produce many file states between two commits. Most are throwaway. Some matter a lot: the last green state before a bad refactor, the approach that almost worked, the attempt another model was making.

Today these states live in three places, none of them right for the job:

1. **Nowhere.** They are overwritten and lost.
2. **The harness's local undo.** Claude Code, Cursor and others can rewind a session. That history is local, tied to one harness and one session, and gone when the machine or container goes away.
3. **Git itself.** WIP commits or side refs. This can work, and Git can store it. But the repository is built around accepted history. WIP states then share its retention, permissions and sync rules, and someone has to clean them up.

The pain is sharpest when many agents work in parallel on many machines. One agent can't pick up from another agent's exact state. A failed run can't be rolled back to its last good point unless someone committed it. A human who gets an escalation can't see what was tried.

ghf's case is **independent lifecycle and control**. Working states get their own retention, authorization, sync and recovery rules, separate from accepted history. ghf does not depend on claims that Git can't handle many small changes.

## 3. Who it's for

**Customer zero: Cloop.** Cloop runs a full ticket-to-PR pipeline with several models, a review loop and human gates, on per-customer machines. It has many agents, many machines and real escalations.

**First external users:** platform owners and engineers who run agent fleets.

**Later, maybe:** solo developers. Their harness's local undo already covers most of their need. We revisit only after the fleet workflow works end to end.

## 4. Core use cases

**U1. Roll back to last green.** A coding agent passes tests, then a later step (a simplification pass, a review fix) breaks them. The pipeline returns to the last checkpoint where tests passed, without a WIP commit.

**U2. Hand off between agents and machines.** Work started by one agent or worker continues on another machine from the exact file state, including uncommitted changes.

**U3. Fork an attempt.** Two models try different approaches from the same checkpoint, each in its own worktree and trace. The pipeline keeps the winner and closes the other.

**U4. Show the human what was tried.** When the review loop escalates to a human, they can list the trace and diff any two points.

**U5. Recover after a crash.** A worker dies mid-task. Locally buffered checkpoints survive the restart and sync when the network returns.

## 5. Why ghf and not the alternatives

| Option | What it does well | Where it falls short for fleets |
|---|---|---|
| Harness local undo | Free, built in, zero setup | Local only, one harness, one session, lost with the machine |
| WIP commits or side refs | Plain Git, nothing new to run | Shares the repo's retention, permissions and sync; needs cleanup; mixes working and accepted history in one system |
| Entire | Multi-harness session capture linked to commits, search, resume | Saves a checkpoint per commit, not per edit. Adds an `Entire-Checkpoint` trailer to commit messages. Stores data in the repo (a Git ref or branch) |
| ghf | Per-edit worktree states, shared across machines, own retention and permissions, zero Git changes | A new tool and a server to run |

The one-line answer: **Entire records the conversation behind each commit. ghf records the file states between commits, and touches nothing in Git.**

## 6. Goals and non-goals

### Goals

- Capture worktree states at agent edit boundaries with no extra work from the agent.
- Make every retained checkpoint restorable on any authorized machine.
- Keep Git history, commits and refs exactly as they would be without ghf.
- Give working history its own retention and permissions.
- Make restore hard to misuse. Never destroy uncaptured work by default.
- Treat the CLI as an automation protocol as well as a human interface.
- Measure everything in Cloop before setting targets.

### Non-goals (MVP)

- Explaining why changes were made (that is GitStory).
- Storing prompts, reasoning or transcripts.
- Seeing every file change on the machine (shell scripts, generators, editors).
- Detecting whether a change was later reversed in behavior.
- Automatic mapping of traces through rebases, squashes and cherry-picks.
- Multiple concurrent writers in one worktree.
- Replacing Git, its storage, or GitHub/GitLab.
- Agent orchestration or merge decisions.
- A local-only permanent mode.

## 7. Concepts

| Term | Meaning | Lifetime |
|---|---|---|
| Checkpoint | A complete, restorable worktree state, plus provenance | Temporary |
| Trace | Ordered list of checkpoints for one unit of work, written by one worktree at a time | Temporary |
| Fork | A new trace that starts from a checkpoint in another trace | Same as a trace |
| Worktree context | A ghf identity for one working directory on one machine | Until removed |
| Hold | A named lock that stops a trace from being pruned | Until released or expired |
| Git commit | Accepted work, made the normal way | Permanent |

### 7.1 Logical versus physical checkpoints

**Logical model.** A checkpoint is a complete worktree state: every captured path and its content at that moment. Restoring it never depends on "replaying" anything the user can see.

**Physical model.** How the server stores that state is an implementation choice and can change. The MVP proposal:

- Each checkpoint has a **manifest**: a tree of path to content hash, structured like Git trees so unchanged directories are shared between checkpoints.
- File content lives in **content-addressed blobs**, compressed and deduplicated.
- A checkpoint is therefore a small manifest plus references to shared blobs. No delta chains in the MVP.

With this model, every checkpoint is independently restorable by design. Deltas or periodic snapshots can come later as an optimization, under the invariant below.

### 7.2 Provenance

Each checkpoint records: repository ID, worktree context ID, trace ID, parent checkpoint, base Git revision, timestamp, author (agent or human ID), changed paths since parent, and manifest root. Optional annotations (a short intent line, a test result such as `tests=pass`) can be attached. They never block capture.

### 7.3 Trace states

```text
open  ->  closed  ->  finalized  ->  pruned
             |            ^
             +-- abandoned --+
```

- **Open:** a worktree is writing to it.
- **Closed:** the agent has finished working. Nothing more is written.
- **Finalized:** the work has entered accepted Git history (`--commit <sha>`), or was explicitly abandoned (`--abandoned`).
- **Pruned:** payloads deleted. Trace metadata may remain as a tombstone.

Retention starts at **finalization**, not closing.

## 8. Invariants

1. Plain Git keeps working. Clone, branch, commit, rebase, push, PRs, CI and IDEs behave the same with or without ghf.
2. ghf writes nothing into Git history: no commits, trailers, notes, refs or objects. Local client state lives in a ghf directory under the Git dir (for example `.git/ghf/`), never in refs or the object store. The tracked `.ghf` file is an ordinary file.
3. Traces never merge. Only normal commits enter accepted history.
4. Capture acknowledges only after local persistence.
5. Checkpoints are immutable. Restores and applies add history, never rewrite it.
6. **Every retained checkpoint is independently restorable**, whatever its physical storage. Pruning and compaction must keep every dependency of a retained checkpoint.
7. **One active writer per worktree.** A worktree context writes to at most one open trace at a time.
8. ghf never destroys uncaptured work by default. Anything that could is opt-in and explicit.
9. No secrets in `.ghf`. A new server must be trusted before any upload.
10. ghf doesn't know about GitStory. Other tools interact through public APIs only (reads, holds).

## 9. Functional requirements

Priority: **P0** = MVP, **P1** = soon after, **P2** = later.

### 9.1 Setup and trust

- **P0.** `git hf init` writes a `.ghf` file with server URL and repository name.
- **P0.** On first contact with a server, the client asks the user to trust it. Trust is stored outside the repo.
- **P0.** Credentials come from the environment or a user config file, never from `.ghf`.

### 9.2 Worktrees and concurrency

- **P0.** Each working directory gets its own **worktree context ID**, generated locally and stored under that worktree's Git dir. Two Git worktrees of the same repo never share a context or a checkpoint stream.
- **P0.** A worktree context holds a writer lock on its open trace. A second writer in the same worktree gets a clear error with a stable code.
- **P0.** Parallel agents use separate Git worktrees. The skill teaches this.
- **P0.** Restoring a trace on another machine, or into another directory, always creates a **new** worktree context. It never assumes the original one still exists.
- **P0.** Copying a worktree directory (for example, cloning a VM) must not produce two live contexts with the same ID. The client detects a path or machine mismatch and creates a new context.

### 9.3 Capture

- **P0.** Automatic capture through harness hooks after each file-editing tool call. Two harnesses at launch. Proposed: Claude Code first, then whichever harness Cloop needs second.
- **P0.** `git hf checkpoint` as a manual fallback for any agent, script or human.
- **P0.** Captures tracked and untracked files. Skips anything matched by `.gitignore` or `.ghfignore`.
- **P0.** Records deletes and renames.
- **P0.** Records the base Git revision. The Git index (staged state) is **not** part of a checkpoint in the MVP; checkpoints capture the working tree only. This is stated in the docs.
- **P1.** Size limit per file, with a clear skip record when exceeded.
- **P1.** Symlinks recorded as links. Binary files stored whole and deduplicated.
- **P2.** File watcher mode for changes made outside hooks.

The docs must say exactly what capture covers. Hooks see tool calls. They don't see shell scripts that write files, code generators, or a human in an editor.

### 9.4 Capture failure modes

A failed checkpoint must never be ambiguous.

- **P0. Best-effort mode (default).** If capture fails, the hook logs a warning, records the failure in the local buffer, and lets the tool call continue. The next successful checkpoint captures the full state anyway, because checkpoints are full states.
- **P0. Strict mode.** Set in `.ghf` or by environment variable. If capture fails, the hook returns a failure with a stable error code, and the harness or pipeline can fail the job. Cloop runs in strict mode.
- **P0.** `git hf status` reports recent capture failures and unsynced checkpoints.

### 9.5 Local buffer

- **P0.** Every checkpoint is written to the local store before the command returns.
- **P0.** Survives process crash and machine restart.
- **P0.** Works with no network. Syncs later.
- **P1.** Local disk cap with a warning, never silent drops.

### 9.6 Sync

- **P0.** Background sync to the server. Every write is idempotent by checkpoint ID.
- **P0.** `git hf sync` forces a sync and reports what is pending.
- **P0.** Blobs upload once (content-addressed).

### 9.7 Inspect

- **P0.** `git hf trace list` lists traces with state.
- **P0.** `git hf trace log [<trace>]` lists checkpoints.
- **P0.** `git hf trace diff <a> [<b>]` diffs two checkpoints, or a checkpoint and the working tree.

### 9.8 Restore and apply

These are two different operations with different safety rules.

**Restore** reconstructs an exact checkpoint state.

- **P0.** Default destination is a **new worktree**: `git hf trace restore <id> --worktree <path>`. If no path is given, ghf creates one next to the repo and prints it. The current worktree is untouched.
- **P0.** The new worktree gets a new worktree context and a new trace, forked from the restored checkpoint.
- **P0.** In-place restore (`--in-place`) is allowed only when the current working tree **exactly matches** the latest checkpoint of its open trace. That proves nothing uncaptured would be lost. Otherwise it refuses with a stable error code and suggests a checkpoint first, or a new worktree. This covers U1 in pipelines without risk.
- **P0.** `--in-place --force` overrides the check. It first takes a checkpoint of the current state, so even a forced restore can be undone.
- **P0.** Every in-place restore is recorded as a new checkpoint in the trace.

**Apply** moves changes onto the current worktree.

- **P0.** `git hf trace apply <a>..<b>` applies the changes between two checkpoints onto the current working tree, as a three-way merge.
- **P0.** Conflicts stop the operation, leave the tree unchanged, and report the conflicting paths in JSON. No partial applies in the MVP.
- **P0.** A successful apply is recorded as a new checkpoint.
- **P1.** `--write-conflicts` to leave standard conflict markers for a human or agent to resolve.

**Fork**

- **P0.** `git hf trace fork <id> [--worktree <path>]` is restore into a new worktree, named for intent. Both commands share the same code path.

### 9.9 Lifecycle and retention

- **P0.** `git hf trace close` marks a trace closed.
- **P0.** `git hf trace finalize --commit <sha>` marks it finalized against an accepted commit. ghf checks only that the commit exists and is reachable from the configured main branch. It does not track rebases or squashes. The pipeline or a human names the landed commit.
- **P0.** `git hf trace finalize --abandoned` finalizes without a commit.
- **P0.** Retention clock: a finalized trace becomes eligible for pruning N days after finalization (default 30, set per repo or org).
- **P0.** **Unfinalized cap:** a closed trace that is never finalized becomes eligible after M days (default 90). ghf warns at M minus 7 days through `git hf status` and the API. This stops forgotten traces from living forever.
- **P0.** Pruning skips any trace with a hold, unsynced checkpoints on a known client, or retained descendants that depend on it. A fork's parent checkpoint stays restorable as long as the fork exists.
- **P0.** Holds: any authorized tool can place and release a named hold with an optional expiry. `git hf trace pin` is a manual hold.
- **P0.** Pruning never touches Git.
- **P1.** Pruning runs as blob-level garbage collection: drop manifests of pruned checkpoints, then delete blobs no retained manifest references.
- **P2.** Optional auto-finalize from Git host webhooks (PR merged or closed).

### 9.10 CLI as an automation protocol

Agents are the primary users. The CLI contract matters more than its looks.

- **P0.** Every command supports `--json`. The JSON output is a **versioned, documented schema**, part of the public API.
- **P0.** Stable, documented error codes (for example `E_WORKTREE_DIRTY`, `E_WRITER_LOCKED`, `E_APPLY_CONFLICT`, `E_NOT_SYNCED`). Codes never change meaning between versions.
- **P0.** Defined exit codes per error class.
- **P0.** Mutating commands are idempotent where possible and accept an `--idempotency-key` where not.
- **P0.** No interactive prompts when `--json` or a non-TTY is detected. Missing input is an error, never a hang.
- **P1.** A machine-readable command reference the skill can load.

### 9.11 Agent skill

- **P0.** Installable skill that teaches agents to: detect ghf, use one worktree per parallel agent, rely on automatic capture, use the manual fallback, prefer restore into a new worktree, use apply for moving changes, fork for experiments, close and finalize traces, and make normal Git commits.
- **P0.** The skill states the capture coverage gap and the staged-changes gap plainly.

### 9.12 Server

- **P0.** Rust service. Authenticated, scoped per repository.
- **P0.** Stores immutable manifests and compressed content-addressed blobs. No custom storage engine for MVP.
- **P0.** APIs: ingest, list, fetch, restore payload, close, finalize, hold, prune.
- **P0.** Self-hostable as one binary plus a data directory.
- **P1.** Org-level retention policy and access roles.
- **P2.** Hosted multi-tenant service.

## 10. Illustrative CLI

Not final.

```bash
git hf init
git hf status
git hf checkpoint [-m "short intent"] [--annotate tests=pass]
git hf sync

git hf trace list
git hf trace log [<trace>]
git hf trace diff <a> [<b>]

git hf trace restore <id> [--worktree <path>]
git hf trace restore <id> --in-place [--force]
git hf trace apply <a>..<b> [--write-conflicts]
git hf trace fork <id> [--worktree <path>]

git hf trace close [<trace>]
git hf trace finalize --commit <sha> | --abandoned
git hf trace pin <trace>
git hf prune [--dry-run]
```

All commands support `--json`.

## 11. Non-functional requirements

Targets are proposals. Milestone 0 replaces them with Cloop numbers.

| Area | Proposed target |
|---|---|
| Capture latency (local ack) | p95 under 50 ms for a typical edit |
| Agent overhead | Under 2% of total agent run time |
| Sync delay when online | p95 under 5 s |
| Cross-machine restore time | p95 under 10 s for a typical repo |
| Restore correctness | 100% byte-identical in tests |
| Data loss | Zero acknowledged checkpoints lost after a crash |
| Storage | Report bytes per checkpoint after dedup and compression; no target until measured |
| Platforms | Linux and macOS at launch; Windows P2 |

**Security and privacy.** TLS everywhere. Repo-scoped auth. No secrets in repo files. `.ghfignore` for paths that must never leave the machine. Encryption at rest on the server is P1.

## 12. Success metrics

Measured in Cloop over the pilot.

**Value**

- Rescues: runs that ended green after a restore or apply, which would otherwise have failed or restarted.
- **Median time saved per rescue**, and the model cost saved where we can estimate it. One rescue of an expensive run may justify a lot of checkpoints.
- Handoffs: tasks continued on a different machine from a checkpoint.
- Escalation use: how often humans open a trace when a run escalates.

**Cost**

- Agent run overhead from capture.
- Storage per task after dedup, and server cost per active repo.
- Share of checkpoints ever used for restore, apply, fork or diff.
- Time to reconstruct a checkpoint on another machine.

**Safety**

- Zero cases of lost uncaptured work from restore or apply.
- Capture failure rate, split by best-effort and strict mode.

**Kill rule.** If time saved stays small compared to overhead and running cost after a month, the problem is smaller than we think and we stop.

## 13. Milestones

**M0. Measure (1 to 2 weeks).** Add a throwaway logger to Cloop runs. Record edits per minute, file sizes, how often runs go red after green, the cost and duration of runs that restart, and how often work moves between workers. Set the targets in section 11.

**M1. Local client.** `git-hf` with worktree contexts, writer locks, capture (one harness plus manual), best-effort and strict modes, local buffer, log, diff, restore (new worktree and guarded in-place), apply, fork. JSON schema and error codes v1. Works on one machine.

**M2. Server and sync.** Rust server, auth, trust flow, background sync, cross-machine restore into a new worktree context.

**M3. Lifecycle.** Close, finalize, unfinalized cap, holds, pin, prune with dependency checks. Second harness adapter. Agent skill.

**M4. Cloop pilot.** Wire U1 to U5 into the Cloop pipeline in strict mode. Run for a month. Review section 12.

## 14. Acceptance scenarios

1. Clone a plain Git repo. Run `git hf init`. Trust the server.
2. Create two Git worktrees. Run one agent in each. Each gets its own worktree context and trace. A second writer in the same worktree is refused with `E_WRITER_LOCKED`.
3. An agent edits files many times. Hooks create checkpoints with no agent action.
4. In strict mode, break the local store. The hook fails the tool call with a stable error code. In best-effort mode, the same failure logs a warning and the run continues.
5. Kill the worker mid-task. Restart. All acknowledged checkpoints are still there.
6. Go offline, keep working, come back. Sync completes with no duplicates.
7. Restore an earlier checkpoint with no flags. It lands in a new worktree. The original worktree is untouched.
8. Make an uncaptured edit, then try `--in-place`. It refuses. Take a checkpoint, retry, it succeeds and is recorded as a new checkpoint.
9. Apply a range onto a worktree with a conflicting edit. It stops, reports the conflict in JSON, and leaves the tree unchanged.
10. Fork from a checkpoint and continue on a second machine in a new worktree context.
11. Prune the parent trace's other checkpoints. The fork's base checkpoint stays restorable.
12. Make a normal Git commit, push, merge through GitHub. Finalize the trace against the merged commit.
13. Wait for eligibility. Prune.
14. A closed trace that is never finalized gets a warning, then becomes eligible at the cap.
15. Git history, refs and commit messages are identical to a run without ghf. A machine without ghf sees nothing different.

## 15. Risks

- **The problem may be small.** Harness undo might be enough. M0 exists to find out early.
- **Capture gaps.** Hooks miss shell writes and generators. If those matter in Cloop, a watcher moves from P2 to P1.
- **Restore and apply safety.** One incident that eats work would kill trust. This gets the most test effort.
- **Manifest size.** Very large repos make per-checkpoint manifests bigger. Tree-structured manifests share unchanged directories, but M0 must check this on Cloop's largest repo.
- **Finalization discipline.** If pipelines forget to finalize, traces sit until the unfinalized cap. Cloop finalizes automatically at merge. Others may need the P2 webhook.
- **Entire adds per-edit capture.** Then our main difference is "nothing in Git" plus separate lifecycle. Watch them.
- **Running a server is a cost for users.** Self-hosting must be one binary.
- **Name clash.** `hf` is also the Hugging Face CLI. Check before launch.

## 16. Decisions made in v0.2

| Area | Decision |
|---|---|
| Checkpoint semantics | Logical full state; physical storage is manifest plus content-addressed blobs; every retained checkpoint independently restorable |
| Restore | Separate restore from apply; restore defaults to a new worktree; in-place only when nothing uncaptured would be lost |
| Retention | Clock starts at finalization; closed-but-unfinalized traces capped at M days |
| Concurrency | One active writer per worktree; parallel agents use separate Git worktrees; cross-machine restore makes a new context |
| Capture failures | Best-effort default, strict mode for pipelines |
| CLI | JSON schema, error codes and exit codes are a versioned public API |
| Staged changes | Not captured in MVP; documented |

## 17. Open questions

1. Trace identity. Proposal: one unit of work, set by the pipeline. Does one Cloop ticket map to one trace, with forks as child traces?
2. Checkpoint boundary. Per tool call is the proposal. Do we need finer granularity inside a call?
3. Default worktree location for restore and fork when no path is given.
4. Default values for N (after finalization) and M (unfinalized cap).
5. Commercial model. Proposal: open-source client, protocol and self-hosted server; paid hosted service later.
6. Cursor Agent Trace: adopt, export, or leave to GitStory?
7. Second harness for launch.
8. Final name and CLI grammar.

## 18. Relationship to GitStory

GitStory is a separate product with a separate PRD. ghf exposes two things it can use: read access to traces, and holds. GitStory places a hold on a trace while it gathers evidence, then releases it. ghf never waits on GitStory directly.
