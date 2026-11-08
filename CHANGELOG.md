# Changelog

## Unreleased

### Fixed

- **Work unit lifecycle.** A work unit's state (`open`, `landed`, `reopened`) is now derived from its landings and its last activity, kept in `meta.json`:
  - `land --since` no longer saves its range and session window, so later drafts and landings of the same work unit see new commits and sessions.
  - A landing closes the round: reconciled drafts move to `drafts/landed/<sha>/`, applied `--supersedes` notes are cleared, and `--since` advances the work unit's base to the landed tip. The next landing no longer replays old drafts or adds a second `supersedes` link.
  - `gc` no longer deletes work that arrived after a landing, and unlanded work expires from its last activity, not from when the work unit was created.
  - Each landing keeps its own time.
  - Work unit ids that map to the same local directory (`feat/a` and `feat_a`) are refused with `E_USAGE`.

### Changed

- `status --json`: each work unit now has `state` and `last_activity`; `landed` is a list of `{commit, story, at}` instead of `[commit, story]` pairs; `expires_at` is when `gc` will actually remove it. Work unit meta written by 0.0.1 still loads.

## 0.0.1 — 2026-11-04

The first public preview. Anything can change before v1, including commands, the `--json` schema and the story format.

### Added

- **Collection.** Evidence from notes, test results, Git, Claude Code session logs, and ghf traces through ghf's public CLI. Each event keeps a durable part (a typed fact, a short scrubbed excerpt and a source hash) and an optional pointer to the original.
- **Scrubbing** of secrets and high-entropy strings, `.storyignore`, and excerpt caps.
- **One egress choke point** with policy, endpoint trust and a manifest per story. `inference = local` refuses any external endpoint.
- **Synthesis** of `gitstory/v1` stories with Anthropic, OpenAI-compatible and command providers. Inferred claims only ever land in hypotheses.
- **Validation** of every authoritative claim against the durable parts of its citations:
  - test claims are checked mechanically against typed test facts;
  - the diff counts as evidence only for claims that cite an edit or commit;
  - a partly supported claim gets one repair from its own evidence that keeps its meaning, then is judged again;
  - verdicts attach by echoed claim text, and a claim the checker never rules on is not kept.
- **Landing reconciliation** after merge or squash merge, with blame-based `supersedes` and `related` link proposals that are never applied on their own.
- **Retrieval:** `blame`, `path`, `history` and `search`, following `supersedes` links to the decision that holds now, plus `--brief` for agents.
- **Stores:** a local store and S3 (SigV4), a rebuildable SQLite index, and export and import.
- **Hooks and skill:** background drafting hooks for Claude Code and Git, and the agent skill.
- **Study tools** for the M0 quality study (hidden from `--help`): `study audit` for the validator audit, and `land --study-baseline` for a blind control arm.

### Benchmark

On a corpus of 119 planted false claims and 111 true ones, Opus 5.5, gpt-6-sol and Haiku 5.5 each caught at least 95% of the false claims and lost at most 5% of the true ones, shown at 95% confidence. See [study/validator-audit/RESULTS.md](study/validator-audit/RESULTS.md).
