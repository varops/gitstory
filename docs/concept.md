# ghf — High-frequency source control for AI agents

**Project concept · v0.4 · 15 October 2026**  
**Status:** Product architecture agreed; implementation contracts and benchmarks pending.

## 1. Executive summary

ghf (Git High Frequency) extends ordinary Git with **high-frequency, recoverable filesystem checkpoints** for autonomous coding agents. Checkpoints form **traces**: temporary, independently addressable histories of experiments, edits, and recovery points. Completed work is integrated through **ordinary Git commits and pull requests**, with no ghf metadata written into Git history.

**GitStory is now a separate, optional product.** It builds durable, evidence-backed engineering memory for commits from agent sessions, tests, Git diffs, and—when available—ghf traces. It is accessed through its own `git story` extension and can work without ghf.

**Product contract:** Git-native or fully Git-compatible drop-in workflows are essential for adoption. The initial implementation extends Git rather than replacing it. Replacing Git's backend remains a possible *future implementation choice*, never a prerequisite for using ghf.

> **Git preserves what changed. ghf captures how work evolved. GitStory preserves why decisions were made.**

## 2. Why this exists

AI coding agents produce many intermediate filesystem states while completing a single meaningful change. Git commits are excellent integration milestones but an awkward place to persist every edit, abandoned hypothesis, and failed experiment. Existing local undo and agent-session resume help, but do not inherently provide an independent, durable, queryable version history across agents, workspaces, and sessions.

ghf provides a high-frequency versioning layer with its own lifecycle, permissions, synchronization, and retention. It is **not** predicated on an unproven claim that Git cannot store thousands of small changes: benchmark real payloads and recovery workloads. The primary distinction is **working history versus accepted history**, not a claim of inevitable Git object bloat.

GitStory addresses a related but independent loss: commit messages and diffs rarely preserve the alternatives considered, tests run, and decisions made during implementation. Its value depends on credible evidence, not plausible narratives inferred from diffs.

## 3. Concepts and invariants

| Artifact | Meaning | Lifecycle |
|---|---|---|
| **Checkpoint** | Recoverable record of filesystem changes, possibly incomplete or failing | Temporary; locally durable, remotely synchronized |
| **ghf trace** | Ordered/forkable sequence of checkpoints for a unit of work, potentially spanning agent sessions | Temporary; independently queryable and recoverable |
| **Git commit** | Conventional accepted integration milestone | Permanent under normal Git semantics |
| **Pull request** | Conventional review/integration workflow | Existing Git hosting |
| **GitStory** | Evidence-backed explanation of engineering decisions associated with a commit | Durable and exportable; independent of traces |
| **Agent Trace record** | Attribution/provenance record in Cursor's specification | Optional interoperability input/output, **not** a ghf trace |

**Locked invariants**

1. **Ordinary Git remains fully usable.** Clone, branch, commit, rebase, cherry-pick, push, PRs, CI, IDEs, and hosting continue working without ghf or GitStory.
2. **No ghf data inside Git commits or history.** No story IDs, commit trailers, Git notes, ghf refs, or checkpoint objects. The tracked `.ghf` configuration is merely an ordinary repository file.
3. **Traces never merge directly.** Only conventional commits enter accepted integration history—even if a future implementation replaces Git's storage engine.
4. **Separate lifecycles.** Checkpoints and traces can expire; commits and GitStories survive independently, subject to explicit repository retention policy.
5. **Independent adoption.** ghf works without GitStory; GitStory works without ghf; human Git users can ignore both.
6. **Agent-agnostic protocol.** Harness-specific adapters are allowed and expected, but neither product depends on one model or agent.
7. **Local durability, remote canonical storage.** ghf buffers offline and recovers locally; the initial product's shared history lives on a separate ghf server, not in a Git branch. No standalone local-only server mode in MVP.
8. **Trust and privacy.** No secrets in repository configuration; upload destinations require trust approval; source, sessions, and stories obey repository authorization and configurable collection policies.
9. **Git compatibility is the external contract; Git backend is replaceable only eventually.** Backend replacement is not an MVP workstream.

## 4. Two independent products

```text
                Developer / coding agent
                         |
            +------------+-------------+
            |                          |
        ordinary Git                agent harness
            |                          |
      Git repo / host          +-------+--------+
      commits and PRs          |                |
            |              file edits      session/events
            |                  |                |
            |           git-hf extension   GitStory adapter
            |                  |                |
            |            local durable      normalized,
            |                buffer        scoped evidence
            |                  |                |
            |              ghf server           |
            |            traces/checkpoints     |
            +------------------+----------------+
                               |
                         GitStory service
                       commit association,
                       synthesis and search
                               |
                         git story HEAD

      Both services operate independently; arrows are optional inputs.
```

### 4.1 ghf: versioned working history

**Deliverables:** Rust `git-hf` extension, independent ghf server, automatic capture adapters, and an agent skill.

Git discovers the executable `git-hf` as the `git hf` subcommand. Illustrative commands:

```bash
git hf init
git hf checkpoint
git hf trace log
git hf trace diff <checkpoint-a> <checkpoint-b>
git hf trace restore <checkpoint-id>
git hf trace fork <checkpoint-id>
git hf sync
```

A checkpoint records actual filesystem changes and essential provenance (repository, worktree, trace, parent, base Git revision, timestamps, changed content references). It **does not require** agent prompts, reasoning, or transcripts. Optional annotations can carry intent or test evidence, but must not block capture.

**Automatic capture should be the default** where a supported agent harness provides post-edit hooks; explicit `git hf checkpoint` remains a universal fallback. Hooks cannot guarantee observation of shell scripts, generators, subprocesses, or external editors. MVP documentation must state capture coverage precisely, rather than promise every filesystem mutation. A later watcher/overlay approach can improve completeness.

**Local durable buffer:** creation acknowledges only after local persistence; remote sync is asynchronous, retryable, and idempotent. Offline recovery must work for locally buffered checkpoints. The server is the canonical shared checkpoint store.

**Remote service:** ingest immutable checkpoints, index traces, deduplicate/compress content, authorize retrieval, synchronize multiple clients, enforce retention, and prune. Use a simple content-addressed blob store and compression initially; choose an optimized storage engine only after measuring real workloads.

### 4.2 GitStory: engineering memory

**Deliverables:** standalone `git-story` extension, optional service/storage backend, evidence adapters, search, and portable export/import. GitStory is not a module that must be installed with ghf.

```bash
git story                  # defaults to HEAD
git story HEAD
git story <commit-sha>
git story search "Why did we remove Redis locking?"
git story export --format markdown
git story export --format json
```

A story is associated **server-side** with a repository identity and conventional commit SHA. The story has its own stable internal identity, but **nothing is inserted into the commit, its message, or Git refs**. The normal commit message must remain useful without GitStory; it can serve as a concise excerpt, not as a pointer or mandatory trailer. Git commit messages have no hard character limit.

**Inputs, when available:**

- Native agent sessions/conversations, ingested under explicit privacy policy; ideally extract engineering events locally rather than upload complete transcripts.
- Git commit metadata, final diffs, and repository history.
- Test commands, outcomes, diagnostics, and tool execution evidence.
- ghf traces/checkpoints, when installed, for evidence of attempted and reverted filesystem changes.
- Explicit human/agent decision annotations.
- Cursor Agent Trace attribution records through an adapter, not as the canonical checkpoint schema.

**Evidence contract:** distinguish observed facts, explicitly stated reasons, and inferred hypotheses. Cite source event IDs or available evidence references internally; never fabricate intent from diffs. When evidence is insufficient, the story must say so. Record known limitations and rejected alternatives only when supported. Access to source evidence and derived search results must respect authorization and privacy boundaries.

**Lifecycle:** collect evidence, associate it with the commit that actually lands in accepted history, generate a draft story, optionally review/correct it, then retain/export it independently of source-session and ghf trace retention. GitStory should support generation from historical agent sessions when reliable commit associations can be established. MVP need not solve general rebases/squashes/cherry-picks automatically: bind to the **landed commit** first; support explicit reassociation later. Never silently attach stories based solely on similar diffs.

**Permanence and portability:** documented Markdown/JSON export, bulk export/import, backups, and stable association metadata. A server-only copy is not sufficient to claim durable organizational memory. Self-hosting and migration should be supported without changing Git history.

## 5. Configuration and trust

A tracked `.ghf` file enables the ghf extension while remaining inert to ordinary Git clients:

```toml
version = 1

[server]
url = "https://ghf.example.com"
repository = "org/project"

[checkpoints]
mode = "auto"   # supported harnesses; explicit fallback
sync = "background"

[retention]
after_merge = "30d"
```

This is illustrative, not a finalized schema. No credentials belong in `.ghf`. A newly encountered server endpoint must be trusted before source uploads; auth is managed outside repository files. GitStory has **independent** service discovery and credentials (configuration format TBD), although shared deployment and identity are allowed.

## 6. Lifecycle and retention

```text
Agent edits -> checkpoints -> ghf trace -> standard Git commit -> PR -> merge
                 |                                   |
           temporary recovery                   optional GitStory
                 |                                   |
            retention/GC                      durable engineering memory
```

1. Capture checkpoint after a supported edit boundary; persist locally before acknowledging.
2. Sync checkpoints to the authorized ghf server; keep independent trace streams for parallel work.
3. Inspect, diff, restore, or fork checkpoints safely without clobbering unrelated working-tree changes.
4. Create **ordinary Git commits** and push/merge through normal Git workflows. ghf never makes a trace directly mergeable.
5. Optionally assemble GitStory from session evidence, tests, Git metadata, and ghf traces while evidence exists.
6. Close/resolve traces; prune eligible temporary payloads while retaining any independent GitStories.

**Initial pruning rule:** configurable N days after a trace is closed **and** its checkpoint data is no longer needed for active work, with explicit pins/holds overriding deletion. When a GitStory is enabled and expected, ensure evidence ingestion or story creation is complete before pruning. The earlier proposal of automatic *semantic reversal detection* is deferred; do not promise to infer whether a change was behaviorally reversed. A 30-day default is a proposal, not a fixed protocol requirement. Pruning must never alter Git commits.

## 7. Interoperability and competition

### Entire — closest direct competitor

Entire provides agent session capture, checkpoints, recovery, and context linked to commits, including multi-harness integrations. Its design stores checkpoint/session material in Git-associated history. ghf's deliberate differences are **filesystem-state-first checkpointing**, an **independent temporary storage lifecycle**, and **no Git-history metadata or commit modifications**. GitStory separately targets portable, evidence-backed engineering memory rather than making complete session capture a requirement.

These are architectural tradeoffs, not proof of superior performance. Entire's Git-based storage can offer portability and operational simplicity; ghf must demonstrate practical advantages in retention, cross-session recovery, permissions, privacy, and high-frequency operation. A PRD should explicitly answer: **why ghf rather than Entire?**

### Cursor Agent Trace — attribution interoperability

Cursor's Agent Trace proposal addresses AI code attribution and provenance, including contributor/session associations with code. It is **not** a filesystem checkpoint protocol and should not replace ghf's trace model. Support import/export via a versioned adapter if the spec and licenses permit. Avoid treating the similarly named concepts as equivalent.

### git-story.dev — distinct product

The existing git-story.dev offering turns repository/commit history into narrative content. Our GitStory is **commit-linked engineering memory grounded in agent and test evidence**, with durable retrieval and search. There is no assumption that the site's implementation is open-source or an appropriate fork base. **GitStory naming, trademark, and CLI conflicts remain an explicit pre-launch check.**

### Anthropic's Scion and FrankenGit — backend context

Both illustrate alternative approaches to agent-scale source-control infrastructure. They are architectural references for storage and concurrency, **not reasons to replace Git in v1**. Keep future backend compatibility as a design constraint, not an implementation milestone.

## 8. MVP and first customer

**Customer zero: Cloop**, using real coding-agent workflows to measure checkpoint frequency, recovery usefulness, story evidence quality, privacy tradeoffs, and storage costs. Initial users are the engineers/platform owners running agent fleets; evaluate broader solo-developer adoption only after the workflow works end-to-end. Commercial packaging and hosted/self-hosted economics are PRD questions, not assumed facts.

**ghf MVP**

- Rust `git-hf` command; Git repo/worktree discovery and trusted `.ghf` configuration.
- Checkpoint capture, local durable buffer, sync/retry, list/diff/restore, simple trace identities.
- Rust server with authenticated repository-scoped storage and retrieval; simple compressed content-addressed blobs.
- One or two real harness hook integrations plus explicit checkpoint fallback.
- Safe restoration and basic pin/close/prune semantics; measurable benchmark harness.
- Installable agent skill explaining checkpoint/recovery workflows, coverage limitations, and conventional Git commit boundaries.

**GitStory MVP (independent track)**

- `git-story` command and simple server/local service interface.
- One agent-session adapter plus Git commit/diff and test-evidence ingestion; ghf trace adapter optional at launch.
- Evidence-backed per-commit story creation, retrieval (`git story HEAD`), basic search, and Markdown/JSON export.
- Explicit privacy controls and commit association to the final landed SHA.

**Deferred:** complete filesystem observation, semantic reversal detection, automated complex rebase/squash reassociation, distributed agent coordination, elaborate storage-engine bake-offs, Git replacement, GitHub/forge replacement, advanced GitStory narrative generation, and full Cursor Agent Trace interoperability if the adapter is not yet stable.

## 9. MVP acceptance scenarios

**ghf:** clone an ordinary Git repo; add `.ghf`; agent modifies files repeatedly; hook captures checkpoints; buffer survives restart/offline period; sync to server; inspect/diff/restore a checkpoint safely; create/push/merge a normal Git commit; close and prune trace data; verify Git history and normal Git clients remain unaffected. A second authorized machine retrieves unpruned checkpoints.

**GitStory without ghf:** use an ordinary repository and a supported agent-session source; associate evidence with a landed commit; generate a story with attributable facts and explicit unknowns; retrieve via `git story HEAD`; search; export/import Markdown or JSON; verify no Git object or ref was modified.

**Together:** generate a GitStory using both session context and ghf checkpoint evidence; prune the ghf trace; verify the GitStory remains readable and portable without the ghf checkpoint server.

**Benchmarks:** record checkpoints/minute, average and p95 payload sizes, storage overhead after deduplication/compression, capture latency, offline recovery, sync time, restore correctness, prune cost, and story evidence coverage. Set numeric SLOs from Cloop measurements, not guesses.

## 10. Questions for the technical specification

- Precise checkpoint boundary: post-tool-call, per file mutation, explicit command, or hybrid?
- Identity and concurrency: repository, worktree, agent, task, trace, parent/fork, and base Git revision.
- Capture coverage: shell edits, generated files, binary changes, renames, deletions, symlinks, ignored paths, secrets, and concurrent writes.
- Restore semantics: conflict safety, untracked/staged state, rollback of restores, and workspace isolation.
- Local buffer durability, idempotent sync, auth, multi-tenant authorization, encryption, and endpoint trust.
- Server storage representation, compression, snapshots, indexing, and pruning state machine.
- Trace close/pin policy, unsynced-client protections, and configurable retention.
- GitStory evidence schema, adapters, privacy filters, provenance, review/correction, commit association, and portability.
- GitStory standalone configuration, service API, hosting and commercial model.
- Cursor Agent Trace adapter compatibility and versioning.
- Harness support matrix and agent skill packaging/installation.
- Product naming and command collisions (`git hf`, `git story`, GitStory trademark).

## 11. Positioning and next step

**ghf:** High-frequency source control for AI agents. Git-compatible by design.

**GitStory:** Evidence-backed engineering memory for Git commits. Independent of ghf; richer when used together.

**Why extend Git rather than replace it?** Because the immediate need is a finer-grained working-history layer and durable engineering context, not migration to a new source-control ecosystem. Preserve commits, PRs, CI, hosting, and familiar commands. Replace internals only if measured requirements eventually justify it.

**Next deliverable:** separate implementation specifications for (A) ghf extension/server/agent skill and (B) GitStory CLI/service/evidence adapters, with an optional integration contract between them. Validate both in Cloop before expanding scope.

---

### Version notes

- **v0.3:** ghf checkpoints/traces plus server-owned GitStory; Git-native adoption and immutable commit boundary.
- **v0.4:** GitStory split into an independent optional service and `git story` extension; agent sessions as primary intent evidence; Cursor Agent Trace interoperability; Entire competitive differentiation; GitStory portability; automatic capture emphasis; simplified retention and Cloop-first MVP.
