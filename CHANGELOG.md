# Changelog

## Unreleased

### Fixed

- **Work unit lifecycle.** A work unit's state (`open`, `landed`, `reopened`) is now derived from its landings and its last activity, kept in `meta.json`:
  - `land --since` no longer saves its range and session window, so later drafts and landings of the same work unit see new commits and sessions.
  - A landing closes the round: reconciled drafts move to `drafts/landed/<sha>/`, applied `--supersedes` notes are cleared, and `--since` advances the work unit's base to the landed tip. The next landing no longer replays old drafts or adds a second `supersedes` link.
  - `gc` no longer deletes work that arrived after a landing, and unlanded work expires from its last activity, not from when the work unit was created.
  - Each landing keeps its own time.
  - Work unit ids that map to the same local directory (`feat/a` and `feat_a`) are refused with `E_USAGE`.

- **Landing is safe to retry.** `land` runs every check that can fail (story rules, blame and link proposals) before it writes anything. When a later step fails, running `land` again finishes the existing story instead of writing a second one for the same commit, which made `validate` and `edit` fail with `E_AMBIGUOUS`.
- **Supersedes cycles.** A `note --supersedes` link that would create a cycle is now skipped at landing and reported. Before, the cycle check had no effect.

- **Story status and links come from the link log only.** `link --remove` now undoes a `supersedes` link that came from `note --supersedes`. `edit --json` and `land --json` show the derived `status`, `supersedes` and `related`, not the stored copy (which always said `current`).
- **Link log order.** Link entries written in the same millisecond by one process keep their order.

- **Link proposals.** Accepting a proposal again after a failure no longer appends a second link. `import` no longer moves an accepted or rejected proposal back to open; it stops with `E_STORE_CONFLICT` instead. Removing the link an accepted proposal created marks the proposal `withdrawn`, a new final status.

- **Validation verdicts.** A verdict is read without regard to case, and any value other than `yes`, `partly` or `no` counts as not judged. Before, anything but `no` or `partly` counted as supported, which let claims through for providers that do not enforce the schema.
- **`validate` and repaired claims.** A claim the repair step fixed is now valid; before, `validate` reported it as unsupported and returned `valid: false`.

- **Concurrent writers on a work unit.** Changes to a work unit's meta now happen under a per-work-unit lock, and each command writes only its own change to the current copy. Before, `land` wrote back the copy it read at the start, losing read offsets, ghf holds and `--supersedes` notes that a background draft or a note wrote meanwhile.

- **ghf holds.** Each hold records when it was placed and is renewed once it is older than a week, so a long-running work unit no longer loses its trace when the 30-day hold expires. `gc` now releases the holds of the work units it removes.

### Changed

- `status --json`: each work unit now has `state` and `last_activity`; `landed` is a list of `{commit, story, at}` instead of `[commit, story]` pairs; `expires_at` is when `gc` will actually remove it. Work unit meta written by 0.0.1 still loads.
- `land --json` adds `resumed` and `skipped_links`.
- `link --json` adds `withdrawn` when removing a link withdraws its proposal. Accepting an already accepted proposal now succeeds without change instead of failing with `E_USAGE`.
- `validate --json` adds `repairs[]`: claims that hold once reworded, as `{from, to}`.
- `export` adds a `stories` list to `gitstory-export.json`: each current story as commands show it. `objects` is unchanged, and bundles without `stories` still import.

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
