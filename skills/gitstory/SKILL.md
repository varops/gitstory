---
name: gitstory
description: Ask why code is the way it is before changing it, and record real decisions as you make them. Use in repositories with a `.gitstory` file, before editing existing code, when you knowingly reverse an earlier decision, and when you make or abandon an approach worth remembering.
---

# GitStory

GitStory keeps the reasons behind landed code: decisions, rejected approaches and open questions. Each claim cites evidence. Use it to learn why code is the way it is before you change it. Record your own decisions as you make them, so the next agent can learn from them too.

Always pass `--json`. GitStory never prompts. Errors carry a stable `error.code`.

## Before changing existing code

```bash
git story blame <path>:<start>-<end> --brief --json
```

This returns the current stories behind those lines. If a story behind the lines was superseded, you get the story that replaced it. For a file whose history matters, also run:

```bash
git story path <path> --brief --json
```

An empty result means nobody recorded a reason. That is not permission to assume there was none.

## How to read a story

- **`decisions`** and **`rejected`** are strong signals. If you are about to undo a decision or retry a rejected approach, read its evidence first. Then either respect it or supersede it on purpose.
- **`unknowns`** are questions the evidence never answered. Do not fill them in by guessing.
- **Hypotheses** are inferred, never verified. Never treat one as a constraint. `--brief` leaves them out.
- A **superseded** story is history. The story that replaced it is the one that holds.
- `git story history <story> --json` shows the full chain of decisions, oldest first.

## Recording decisions

Record a decision when you make it, in one sentence: what you chose and why.

```bash
git story note "Use row versioning instead of a Redis lock because CI has no Redis" --type decision --json
git story note "Retries must stay idempotent: clients resend on timeout" --type constraint --json
git story note --test "cargo test -p billing" --result pass --json
```

- `--type reason` explains a change. `--type constraint` records a rule the code must keep.
- When you abandon an approach, note what you tried and why it failed. Rejected approaches are among the most useful things to record.
- Notes are evidence, not the story. GitStory builds stories from notes, session logs, tests and Git. Do not write a story yourself.

## Changing a past decision

When you knowingly reverse what a story says, record why and name the story you are replacing:

```bash
git story note "Bring the Redis lock back: versioning missed cross-row writes" --supersedes st_01... --json
```

Never edit an old story to fit a new change. Edits are for corrections, and every edit is versioned.

## Drafts and landing

- `git story draft --json` builds a draft from the evidence collected so far. Drafts stay on this machine. Hooks can do this in the background (`git story hooks install claude-code`).
- `git story land --work-unit <branch> --to <sha> --json` runs after the work merges. It checks the drafts against the code that actually landed and writes the story to the shared store. Approaches dropped in review end up in `rejected` or `discarded`, never in `decisions`.
- Landing may propose `supersedes` links. Review them with `git story proposals --json`, then run `git story link --accept <id>` or `git story link --reject <id>`. A proposal changes nothing until someone accepts it.

## Errors worth handling

| Code | Meaning | What to do |
|---|---|---|
| `E_NOT_INITIALIZED` | No `.gitstory` | Ask before running `git story init` |
| `E_NO_EVIDENCE` | Nothing to build from | Record notes first |
| `E_UNTRUSTED_ENDPOINT` | Model endpoint not trusted on this machine | Ask the user to run `git story trust` |
| `E_EGRESS_DENIED` | Policy forbids sending evidence there | Do not work around it |
| `E_COMMIT_NOT_REACHABLE` | Landing a commit not on main | Land after the merge |
| `E_LINK_CYCLE` | The link would loop | Check `history` |
