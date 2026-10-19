---
name: gitstory
description: Ask why code is the way it is before changing it, and record real decisions as you make them. Use in repositories with a `.gitstory` file, before editing existing code, when you knowingly reverse an earlier decision, and when you make or abandon an approach worth remembering.
---

# GitStory

> Status: outline. The commands below are not built yet; sections follow PRD §9.11.

## Before changing existing code

TODO: run `git story blame <path>:<lines> --brief --json` and, for files whose history matters, `git story path <path>`.

## How to read a story

TODO:
- Treat implemented decisions and rejected options as strong signals.
- Treat hypotheses as unverified, never as constraints. `--brief` omits them.
- A superseded story is history; the current story that replaced it is what holds.

## Recording decisions

TODO:
- `git story note "<what and why>" --type decision|reason|constraint` when you make a real decision.
- `git story note --test "<cmd>" --result pass|fail` for test evidence.
- Note abandoned approaches and why.

## Changing a past decision

TODO: when you knowingly reverse what a story says, record it with `git story note "<why>" --supersedes <story>`. Never edit an old story to fit a new change.
