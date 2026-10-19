# GitStory M1 plan

**Status:** draft, 19 October 2026 · **Source:** [GitStory PRD v0.2](gitstory-prd.md) §13

M1 is the local CLI: `git story` with init, notes, Claude Code and Git adapters, test events, durable evidence, scrubbing, egress policy and manifest, synthesis, validation, draft, show, a local store and export. Landing, links and search are M2.

**The risk sits in synthesis, and M0 owns that question.** The PRD puts M0 (hand-built stories for 20 Cloop tasks, rated by humans, with a kill rule) before any product code. This plan splits M1 accordingly:

- **Phases A and B** don't depend on M0's findings, and M0 needs them anyway. M0 has to build evidence bundles by script, so the scrubber, the Claude Code adapter and the Git adapter can be built once, as product code, and used by M0 itself.
- **Phase C** (prompts, model choice, the validation pass) waits for M0's ratings, so we don't harden a prompt the pilot would reject.

## Phase A: evidence and storage (independent of M0)

| # | Work | Notes | Size |
|---|---|---|---|
| A1 | `git story init`, `.gitstory`, `.storyignore`, repo discovery | TOML with store, collection and egress sections (PRD §9.1, §9.3). No credentials. Discovery and config patterns copied from ghf, not imported. | S |
| A2 | Local store | Plain JSON files under `.git/story/`: stories, cited events, egress manifests, link log. Keyed by repo id and commit SHA. Files keep it portable and S3-ready for M2. | M |
| A3 | Search index | SQLite FTS, rebuildable from the store at any time; nothing authoritative lives only here. | S |
| A4 | Evidence model, ids and work units | Events with durable and external parts (`proto/src/evidence.rs`). M1 work unit = `--work-unit`, else the current branch. Collected under `.git/story/collect/<work-unit>/`. | S |
| A5 | `git story note` | Decision, reason and constraint notes; `--test <cmd> --result pass\|fail` as a typed test fact. | S |
| A6 | Git adapter | Commit metadata, diff hunks, changed paths, reverts. | M |
| A7 | Scrubber | Common key formats, `.env` values, tokens, high-entropy strings; per-event excerpt cap; `.storyignore` keeps paths only; `excerpts = none` mode. Tested against a planted-secret corpus (acceptance 4). | M |
| A8 | Claude Code session adapter | Reads local session JSONL. Extracts goal, attempts, abandons, test runs (command and exit status) and stated reasons into durable events. Never uploads the raw log. Versioned parser with fixtures from real, scrubbed logs. | L |
| A9 | Show and export | `git story [<rev>]` as Markdown, `--brief` and `--json`; `export --format markdown\|json`; `import`. | M |

## Phase B: egress (independent of M0)

| # | Work | Notes | Size |
|---|---|---|---|
| B1 | Egress choke point | One module builds every outbound payload from the evidence bundle, applying the PRD §9.3 policy. Nothing else in the codebase may make network calls. `inference = local` refuses any non-local endpoint (acceptance 6). | M |
| B2 | Egress manifest | Per story: endpoint, model, event ids and diff hunks sent, timestamps. Stored and exported with the story (acceptance 5). | S |
| B3 | Endpoint trust and credentials | A trust prompt on first contact, credentials outside the repo; same pattern as ghf. | S |

## Phase C: synthesis and validation (shaped by M0)

| # | Work | Notes | Size |
|---|---|---|---|
| C1 | Model providers | An external provider (Anthropic) and a local OpenAI-compatible endpoint (for example Ollama), behind one interface. | M |
| C2 | Synthesis | Evidence bundle in, `gitstory/v1` out; schema-checked, retried, then a stable error. Inferred claims only in `hypotheses`; a missing reason becomes an unknown. | L |
| C3 | Validation pass | Separate from synthesis. Checks every claim against the durable parts of its citations only. Test claims are checked mechanically against the typed fact. Unsupported claims are dropped, or moved to hypotheses. `Story::violations` enforces the structural invariants. | L |
| C4 | `git story draft` | Manual, plus an optional post-commit hook that drafts in the background, like ghf's sync, so commits never wait (PRD §11). | M |

## Contract and quality (throughout)

- **JSON contract v1:** envelope `gitstory.cli/v1`, stable error and exit codes, a `docs/cli.md` reference tested against the code (as in ghf). Expected new codes: `E_EGRESS_DENIED`, `E_UNTRUSTED_ENDPOINT`, `E_AUTH`, `E_MODEL`, `E_SYNTHESIS_INVALID`, `E_COMMIT_NOT_FOUND`.
- **Acceptance tests for M1:** PRD §14 scenarios 1, 4, 5, 6, 8 and 15, plus a draft built from a recorded Claude Code session. Scenarios 2–3, 7 and 9–14 need landing, links or the shared store, and belong to M2–M3.
- **Agent skill:** filled in as the commands land.

## Decisions this plan makes

| Decision | Choice | Why |
|---|---|---|
| Repository | Separate private `varops/gitstory` | Independence invariant (PRD §8.11); name not yet cleared |
| Store format | Plain JSON files + rebuildable SQLite index | Portable export; S3 backend in M2 is a file copy |
| M1 commit association | A draft attaches to the commit that triggered it (`HEAD`), marked as a draft | Plain-Git users get value before M2's `land` and reconciliation |
| Code shared with ghf | Copied, not imported | Keeps the products independent; about 200 lines |

## Needs your input

- [ ] **Run M0 now, in parallel?** I recommend yes: build A7 and A8 first and use them as M0's bundle scripts.
- [ ] **Sample Claude Code session logs from Cloop** for adapter fixtures, scrubbed. A8 depends on real logs.
- [ ] **Default external model and local option** (PRD open question 1). I'd decide after M0 compares quality and cost.
- [ ] **Excerpt cap per event** (PRD open question 2). Also settled by M0's auditor ratings.

## Order

1. A1, A4, A5: init, events, notes. The CLI becomes usable.
2. A7, A8, A6: the scrubber and adapters. These are what M0 needs.
3. B1–B3: egress, before any model call exists.
4. A2, A3, A9: store, index, show and export.
5. C1–C4, once M0's ratings are in.
