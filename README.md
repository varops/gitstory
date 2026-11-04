# GitStory

**Evidence-backed engineering memory for Git commits.**

> [!IMPORTANT]
> GitStory is an early preview (0.0.1). Anything can change before v1, including commands, the `--json` schema and the story format.

GitStory keeps the "why" behind landed code. It collects evidence from agent sessions, tests, reviews and Git while work happens. It distills that evidence into a short record of decisions and checks every claim against the code that actually landed. The result is linked to the landed commit, so future agents and humans can ask why code looks the way it does, for a commit, a file or a line.

- **Distilled decisions, not transcripts.** Raw session logs never leave the machine by default.
- **Every claim is cited.** Authoritative claims stand on durable evidence alone, so they stay auditable after the sources are gone.
- **Guesses stay labeled.** Inferred claims live apart from decisions and need review before anyone treats them as a constraint.
- **Memory stays current.** Explicit `supersedes` links mark which decision holds now.
- **Nothing in Git.** GitStory writes no commits, trailers, notes or refs, and works with or without [ghf](https://github.com/varops/ghf).

## Status

M1 from the [plan](docs/plan-m1.md) is built, along with the landing, link and shared-store work the PRD puts in M2 and M3. All 15 PRD acceptance scenarios pass against a stand-in model. A first end-to-end run against OpenAI `gpt-6-luna` (draft, squash-merge landing, blame, re-validation) works, and the validator audit below ran live through the `openai` provider with `gpt-6-sol`. The Anthropic provider has not been called live yet. Next comes the M0 quality study: real Claude Code sessions, and auditors rating the stories. The [study tools](study/README.md) are ready: a validator audit with planted false claims, and a control arm that rates each story blind against one plain model call on the same evidence.

Built:
- **Collection:** notes, test results, Git, Claude Code session logs, and ghf traces through ghf's public CLI.
- **Scrubbing** of secrets and high-entropy strings, `.storyignore`, and excerpt caps.
- **One egress choke point** with policy, endpoint trust and a manifest per story. `inference = local` refuses any external endpoint.
- **Synthesis and validation** with Anthropic, OpenAI-compatible and command providers. Test claims are checked mechanically against typed test facts.
- **Landing reconciliation** after merge or squash merge, with blame-based link proposals.
- **Retrieval:** `blame`, `path`, `history` and `search`, which follow `supersedes` links to the decision that holds now.
- **Stores:** a local store and S3 (SigV4), a rebuildable SQLite index, and export and import.
- **Hooks and skill:** background drafting hooks for Claude Code and Git, and the agent skill.

## Benchmark

GitStory's promise is that every claim in a story is supported by its evidence. The [validator audit](study/validator-audit/RESULTS.md) tests that promise: it plants 119 false claims among 111 true ones, in 21 realistic cases, and runs the real validation pass over them.

| Validation model | False claims caught | True claims lost |
|---|---|---|
| Claude Opus 5.5 | 119/119 [0.969, 1.000] | 0/111 [0.000, 0.033] |
| gpt-6-sol (2 runs) | 119/119 [0.969, 1.000] both | 0/111 [0.000, 0.033] both |
| Claude Haiku 5.5 | 118/119 [0.954, 0.999] | 0/111 [0.000, 0.033] |

With every model, at least 95% of false claims were caught and at most 5% of true claims lost, shown at 95% confidence (Wilson intervals in brackets). The corpus covers 14 kinds of falsehood, from wrong numbers to a "because" nobody stated.

It is our own synthetic corpus, so it shows the validator enforces evidence; whether real stories are accurate and useful is what the M0 study measures next. Results, method and limits are in [RESULTS.md](study/validator-audit/RESULTS.md). Run it against your own model with `git story study audit study/validator-audit/corpus.json` (see [study/README.md](study/README.md)).

## Quick start

```bash
git story init                     # writes .gitstory; commit it
git story trust                    # allow the configured model endpoint
git story hooks install claude-code
git story note "Use row versioning instead of a Redis lock because CI has no Redis"
git story draft                    # local draft from the evidence so far
# … merge the branch …
git story land --work-unit feat/retry --to origin/main
git story blame src/lock.rs:40-80 --brief --json
```

## Layout

| Path | Crate | What it is |
|---|---|---|
| [`cli/`](cli) | `git-story` | Client binary. Git runs it as `git story`. |
| [`proto/`](proto) | `gitstory-proto` | CLI envelope, error codes, `gitstory/v1` story and evidence schemas |
| [`skills/gitstory/`](skills/gitstory) | | Agent skill, embedded in the binary (`git story skill install`) |
| [`docs/`](docs) | | CLI reference, PRD, concept, M1 plan |

## Build

Requires Rust 1.89 or later.

```bash
cargo build
cargo test
```

## Automation contract

Every command takes `--json` and prints one versioned envelope:

```json
{"schema":"gitstory.cli/v1","command":"blame","ok":true,"data":{"stories":[]}}
```

Error and exit codes follow the same classes as ghf. GitStory never prompts. The full contract is in the [CLI reference](docs/cli.md), and a test checks that the reference matches the code.

## Docs

- [CLI reference](docs/cli.md)
- [GitStory PRD](docs/gitstory-prd.md)
- [Concept](docs/concept.md), shared with ghf
- [M1 plan](docs/plan-m1.md)
