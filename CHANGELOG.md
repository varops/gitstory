# Changelog

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
