# GitStory

**Evidence-backed engineering memory for Git commits.**

> [!IMPORTANT]
> Pre-M1 and private. The name is a working title until the trademark and CLI-name check in the PRD is done.

GitStory keeps the "why" behind landed code. It collects evidence from agent sessions, tests, reviews and Git while work happens. It distills that evidence into a short record of decisions and checks every claim against the code that actually landed. The result is linked to the landed commit, so future agents and humans can ask why code looks the way it does, for a commit, a file or a line.

- **Distilled decisions, not transcripts.** Raw session logs never leave the machine by default.
- **Every claim is cited.** Authoritative claims stand on durable evidence alone, so they stay auditable after the sources are gone.
- **Guesses stay labeled.** Inferred claims live apart from decisions and need review before anyone treats them as a constraint.
- **Memory stays current.** Explicit `supersedes` links mark which decision holds now.
- **Nothing in Git.** GitStory writes no commits, trailers, notes or refs, and works with or without [ghf](https://github.com/varops/ghf).

## Status

Scaffolding only. The command grammar from the PRD exists, and every command returns `E_NOT_IMPLEMENTED`. The `gitstory/v1` story and evidence schemas are defined in [`proto/`](proto) and tested against the PRD's example. The milestones are set out in the [M1 plan](docs/plan-m1.md).

## Layout

| Path | Crate | What it is |
|---|---|---|
| [`cli/`](cli) | `git-story` | Client binary. Git runs it as `git story`. |
| [`proto/`](proto) | `gitstory-proto` | CLI envelope, error codes, `gitstory/v1` story and evidence schemas |
| [`skills/gitstory/`](skills/gitstory) | | Agent skill (outline) |
| [`docs/`](docs) | | PRD, concept, M1 plan |

## Build

Requires Rust 1.89 or later.

```bash
cargo build
cargo test
```

## Automation contract

Every command takes `--json` and prints one versioned envelope:

```json
{"schema":"gitstory.cli/v1","command":"blame","ok":false,
 "error":{"code":"E_NOT_IMPLEMENTED","message":"`blame` is not implemented yet"}}
```

Error and exit codes follow the same classes as ghf. With `--json` or no TTY, GitStory never prompts.

## Docs

- [GitStory PRD](docs/gitstory-prd.md)
- [Concept](docs/concept.md), shared with ghf
- [M1 plan](docs/plan-m1.md)
