# CLI reference

`git story` is the `git-story` binary. Git finds it on `PATH`.

## Envelope

With `--json`, every command prints exactly one JSON object on stdout:

```json
{"schema": "gitstory.cli/v1", "command": "blame", "ok": true, "data": {}}
{"schema": "gitstory.cli/v1", "command": "land", "ok": false,
 "error": {"code": "E_COMMIT_NOT_REACHABLE", "message": "…"}}
```

- `schema` changes only when the shape changes incompatibly.
- `command` is the stable command name, `show` for a bare `git story [REV]`.
- Human output (without `--json`) is for people and may change. GitStory never prompts, with or without `--json`.

`--brief` switches story views to the agent form: summary, decisions, rejected options, unknowns and links. It leaves out hypotheses, discarded claims and evidence, and list commands skip superseded stories.

## Exit codes

| Code | Class |
|---|---|
| 0 | Success |
| 1 | Internal error |
| 2 | Usage or configuration |
| 3 | Refused by a rule (policy, reachability, cycles) |
| 4 | Conflict |
| 5 | Remote: model, store, credentials or trust |
| 6 | Not found |
| 64 | Not implemented |

## Error codes

| Code | Exit | Meaning |
|---|---|---|
| `E_INTERNAL` | 1 | Unexpected internal failure |
| `E_USAGE` | 2 | Invalid arguments or missing input |
| `E_NOT_A_REPO` | 2 | Not inside a Git worktree |
| `E_NOT_INITIALIZED` | 2 | No `.gitstory`; run `git story init` |
| `E_CONFIG` | 2 | `.gitstory` or user configuration is invalid |
| `E_AMBIGUOUS` | 2 | A story id prefix matches more than one story |
| `E_NOT_FOUND` | 6 | No such story, proposal or file |
| `E_COMMIT_NOT_FOUND` | 6 | The commit does not exist locally |
| `E_COMMIT_NOT_REACHABLE` | 3 | The commit is not reachable from the main branch |
| `E_NO_EVIDENCE` | 3 | The work unit has no evidence yet |
| `E_EGRESS_DENIED` | 3 | The egress policy forbids the request; nothing was sent |
| `E_LINK_CYCLE` | 3 | The link would create a supersession cycle |
| `E_STORE_CONFLICT` | 4 | An import would overwrite different data |
| `E_UNTRUSTED_ENDPOINT` | 5 | The model endpoint is not trusted on this machine |
| `E_AUTH` | 5 | Missing or rejected credentials |
| `E_REMOTE` | 5 | The model or a remote store failed |
| `E_MODEL_REFUSED` | 5 | The model declined the request |
| `E_SYNTHESIS_INVALID` | 1 | The model's output was not a valid story after retries |
| `E_NOT_IMPLEMENTED` | 64 | Reserved |

## References

- **Revisions** are anything Git resolves to a commit: `HEAD`, a branch, a SHA.
- **Story ids** look like `st_01m5…`. Any unique prefix works, with or without `st_`.
- **Work units** default to `$GITSTORY_WORK_UNIT`, then the current branch, then `detached-<sha>`.

## Records

A **story** (`gitstory/v1`) is the unit everything returns:

```json
{"story_id": "st_…", "schema": "gitstory/v1", "repo": "acme/billing", "commit": "<sha>",
 "contains": ["<branch commit sha>"], "work_unit": "feat/retry", "stage": "landed|draft",
 "version": 1, "created_at": 1760540000000, "status": "current|superseded",
 "supersedes": ["st_…"], "related": [], "summary": "…",
 "goal": {"claim": "…", "type": "stated", "evidence": ["ev_…"]},
 "decisions": [{"claim": "…", "type": "observed|stated", "status": "implemented", "evidence": ["ev_…"]}],
 "rejected": [], "discarded": [], "evidence_summary": [],
 "hypotheses": [{"claim": "…", "type": "inferred", "evidence": ["ev_…"]}],
 "limitations": [], "unknowns": ["…"], "paths": ["src/lock.rs"],
 "review": {"status": "unreviewed|reviewed|flagged"}, "egress": "egr_…"}
```

- `contains` lists the commits a merge brought in. Blame on any of them finds the story.
- `status`, `supersedes` and `related` come from the link log only, never from a guess or from the stored copy of the story. Removing a link (`link --remove`) always takes effect, including a link that came from `note --supersedes`. Every command that returns a story, including `land`, `edit` and `export`, shows it this way.
- Every claim outside `hypotheses` cites evidence and is `observed` or `stated`. Inferred claims appear only in `hypotheses`.

An **evidence event** has a durable part that is stored with the story, and an optional external pointer that may stop resolving:

```json
{"id": "ev_…", "type": "goal|decision|reason|attempt|abandon|test|constraint|review",
 "work_unit": "feat/retry",
 "durable": {"fact": "test: cargo test -> pass, 12/12 passed",
             "test": {"command": "cargo test", "passed": 12, "total": 12},
             "excerpt": "scrubbed, capped text", "source_hash": "<blake3>",
             "source": "agent_session|test_run|review|ticket|git|ghf_trace|human_note",
             "at": 1760540000000, "author": "claude-code:<session>"},
 "external": {"pointer": "claude-code:<session>#L42"}, "paths": ["src/lock.rs"]}
```

An **egress manifest** (`egr_…`) lists each model call: `purpose` (`synthesis`, `reconciliation` or `validation`), `provider`, `endpoint`, `model`, `local`, `at`, the `event_ids` sent, the diff `hunks` sent (path and hash), whether `excerpts` were sent, and the request size in `bytes`. A call is recorded before it is sent, so a failed call still appears.

A **link proposal** (`lp_…`) has `kind` (`supersedes` or `related`), `from`, `to`, `reason` and `status`: `open`, `accepted`, `rejected` or `withdrawn`. The only moves are `open` to `accepted` or `rejected`, and `accepted` to `withdrawn` (its link was removed). `accepted`, `rejected` and `withdrawn` are final.

## Commands

### `init [--repository OWNER/NAME] [--trust] [--force]`

Writes `.gitstory` with the defaults below and opens the store. The repository id defaults to the `origin` remote, then `local/<directory>`. `--trust` trusts the configured model endpoint. An existing file is kept unless `--force` is given.

Data: `{path, created, repository, store, model{provider, name, endpoint, local, trusted}, egress}`.

### `trust [ENDPOINT]`

Trusts an endpoint, or every configured non-local endpoint, to receive evidence from this machine. Trust lives in the user config directory, never in the repository. Data: `{trusted[]}`.

### `[REV]` · `show [REV]`

Stories for a commit (default `HEAD`) or a story id. When a commit has no landed story, drafts from this machine are shown. Data: `{commit, stories[]}`.

### `note [TEXT] [--type decision|reason|constraint] [--supersedes STORY] [--work-unit W]`
### `note --test CMD --result pass|fail`

Records a note as stated evidence in the work unit. `--supersedes` marks the story this work replaces; landing turns it into a link. Data: `{work_unit, event, supersedes[]}`.

### `draft [--work-unit W] [--background]`

Collects evidence from Git, Claude Code session logs and ghf (when enabled), builds a draft against `HEAD` and validates it. Each ghf trace it reads gets a `gitstory` hold for 30 days, renewed by any collection once it is older than a week, and released when the work unit lands or `gc` removes it. Drafts and their egress manifests stay in the local collection. `--background` returns at once and drafts in a detached process; output goes to `.git/story/draft.log`.

Data: `{story, work_unit, new_events, events, egress}`.

### `land [--work-unit W] [--to REV] [--since REV]`

Run after the work merges. `--to` (default `HEAD`) must be reachable from the main branch. Landing collects the evidence once more and reconciles the drafts against the landed diff. A claim the landed code does not reflect becomes `rejected` or `discarded`. Landing then validates the story and writes it to the store, with the cited events and the egress manifest. Notes with `--supersedes` become links. A link that would create a supersession cycle is skipped and listed in `skipped_links`. Blame on the lines the commit changed proposes further links.

Every check that can fail runs before anything is written, and the story is written after its evidence and manifest. If a later step fails (a link, a proposal, the local record), run `land` again: when the work unit already has a story for the commit, `land` finishes the remaining steps for that story (`resumed: true`) and never writes a second one. Landing the same commit twice is safe for the same reason.

`--since REV` lands work that went straight to main as several commits: the story covers `REV..TO`, and session evidence counts only between `REV`'s commit time and ten minutes after `TO`. The range and the time window apply to that landing only and are never saved.

A successful landing closes the work unit's round. The drafts it reconciled move to `drafts/landed/<sha>/` in the local collection, and the `--supersedes` notes it applied are cleared. After `--since`, the work unit's collection starts at `TO`. So the next landing of the same work unit (often `main`) starts fresh: it never replays earlier drafts or links. Drafts of a commit the work unit already landed are ignored.

Before synthesis, session evidence is narrowed to the task behind the change. The task starts with the last request before the first session edit to a changed file, and earlier session events are dropped. This applies to `draft` too.

Data: `{story, proposals[], cited_events, egress, study_sheet, resumed, skipped_links[]}`.

### `blame PATH[:START[-END]]`

Stories behind the lines at `HEAD`, through Git blame, following accepted `supersedes` links forward to the stories that hold now. Each entry is `{story, current, via}`. `via` is `blame`, or the id of the superseded story that led to this one. With `--brief`, only current stories are listed.

Data: `{path, range, commits[], stories[]}`.

### `path PATH`

Every story whose paths include `PATH` or lie under it. Current stories come first, then newest first. Data: `{path, stories[]}`.

### `history STORY`

The whole supersession chain around a story, oldest first. Data: `{story, chain[]}`.

### `search QUERY`

Full-text search over summaries, claims, goals and paths. Terms are combined with OR, and current stories come first. Data: `{query, stories[]}`.

### `validate [REV|STORY]`

Re-checks a landed story using only durable evidence: the durable parts of the events it cites, and the commit's diff from Git. Session logs, CI links and ghf traces may be gone. Nothing is changed; any model call is recorded in a new egress manifest.

It runs the same validation as landing, repair step included. A partly supported claim that the repair step rewrote into a supported one holds, and is listed in `repairs[]` as `{from, to}`; the stored story is not changed. A claim that does not hold is listed in `unsupported[]`.

The checker's verdict is read without regard to case. A verdict other than `yes`, `partly` or `no`, like a missing one, counts as not judged (`partly`), so a provider that does not enforce the response schema never gets a claim through by accident.

Data: `{story, valid, unsupported[], repairs[], missing_events[], egress}`.

### `link STORY --supersedes|--related OTHER [--remove]` · `link --accept|--reject PROPOSAL`

Adds or removes a link in the append-only link log, or decides a proposal. A `supersedes` link that would create a cycle is refused. Data: `{link, withdrawn}` or `{proposal}`.

- `--accept` writes the link, then marks the proposal `accepted`. If it fails in between, run it again: a link from that proposal that is already in force is not added twice. Accepting an accepted proposal (or rejecting a rejected one) changes nothing; any other move from a final status fails with `E_USAGE`.
- `--remove` on a link that came from an accepted proposal also marks that proposal `withdrawn`, returned in `withdrawn`.
- Two machines deciding the same proposal at the same moment on a shared S3 store can still race: proposals are not written with conditional requests yet.

The log replays in order of each entry's `at` (unix ms). Within one process `at` strictly increases, so an add and a remove made in the same millisecond keep their order. Entries written on different machines are ordered by those machines' clocks.

### `proposals`

Open link proposals. Data: `{proposals[]}`.

### `edit [REV|STORY] [--story S] [--summary T] [--add-decision T]... [--add-unknown T]... [--promote N]... [--demote N]... [--review reviewed|flagged|unreviewed]`

Edits a landed story as a new version; earlier versions are kept. The edit itself becomes a stated `review` event, and added or promoted decisions cite it. `--promote N` turns hypothesis N (0-based) into a stated decision. `--demote N` turns decision N into a hypothesis.

Data: `{story, event}`.

### `export [--format json|markdown] [--out DIR]`

Writes `gitstory-export.json` (schema `gitstory-export/v1`). Its `objects` hold every stored object as stored, including story versions, cited events, egress manifests, the link log and proposals; `import` reads only these. Its `stories` list each current story as commands show it, with `status`, `supersedes` and `related` from the link log. `markdown` also writes `index.md` and one page per story. Data: `{directory, files, objects}`.

### `import DIR|FILE`

Imports an export. Every object is checked before anything is written. An object that already exists with different content stops the import with `E_STORE_CONFLICT`, except an `open` proposal, which takes the imported status. A proposal that is already `accepted`, `rejected` or `withdrawn` is never changed by import: a different status in the export stops the import with `E_STORE_CONFLICT`. Importing the same export twice changes nothing.

Data: `{written, unchanged, repository}`.

### `status`

Store, model, egress policy and local work units, with warnings for unlanded work close to expiry and untrusted endpoints. Data: `{repository, store, stories, open_proposals, work_units[], model, egress, warnings[]}`.

Each work unit is `{work_unit, state, events, drafts, landed[], created_at, last_activity, expires_at}`:

- `state` is `open` (never landed), `landed` (landed, nothing new since) or `reopened` (landed, then new evidence, notes or drafts arrived).
- `landed[]` lists every landing as `{commit, story, at}`.
- `last_activity` is the time of the last new evidence, note or draft.
- `expires_at` is when `gc` removes the local collection: a day after landing for a `landed` work unit, else `drafts.expire` after `last_activity`.

### `gc [--dry-run]`

Removes the local collection of work units past their `expires_at` (see `status`): landed work units with nothing new for a day since they landed, and work units with unlanded work and no activity for `drafts.expire`. A work unit that got new evidence or drafts after it landed is kept like unlanded work. Landed stories and their cited evidence stay in the store. Data: `{dry_run, removed[]}`.

### `hooks install|uninstall|status claude-code|git [--scope project|local|user]`

- **`claude-code`** adds a `Stop` hook to `.claude/settings.json` (or `settings.local.json`, or the user settings).
- **`git`** adds a `post-commit` hook.

Both run `git story draft --background --json`, so neither waits for the model. Data: `{path, changed, installed, command}`.

### `skill install|show [--scope project|user]`

Installs the agent skill as `.claude/skills/gitstory/SKILL.md`, or prints it. Data: `{path, changed}` or `{skill}`.

### Study tools (hidden)

`study audit CORPUS` and `land --study-baseline DIR` measure GitStory for the M0 study and are hidden from `--help`. See [study/README.md](../study/README.md).

## Configuration: `.gitstory`

Commit this file. It holds no credentials.

```toml
version = 1
repository = "acme/billing"      # optional; default from the origin remote

[store]
kind = "local"                   # local | s3
# path = "/shared/stories"       # local: default .git/story/store
# bucket = "…"                   # s3: with endpoint, region, prefix

[collection]
claude_code = true               # read ~/.claude/projects session logs
ghf = true                       # read ghf traces through `git hf`, hold them until landing
excerpt_cap = 400                # characters per excerpt
excerpts = "on"                  # on | none (typed facts only)

[egress]
inference = "external"           # external | local (refuse non-local endpoints)
send_events = true
send_diff = true
send_excerpts = true

[model]
provider = "anthropic"           # anthropic | openai | command
name = "claude-opus-5-5"
# url = "…"                      # default https://api.anthropic.com or https://api.openai.com/v1
# command = "…"                  # command: JSON request on stdin, JSON on stdout
local = false                    # command: does it stay on this machine?
effort = "high"

# [validation]                   # a separate validation model; provider = "none" skips the model check
# [git]
# main = "origin/main"
[drafts]
expire = "30d"
```

`.storyignore` uses `.gitignore` syntax. Events that touch matching paths keep the path but no content, and matching diff hunks are never sent.

## Environment

| Variable | Effect |
|---|---|
| `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN` | Anthropic credentials |
| `OPENAI_API_KEY` | Credentials for OpenAI or an OpenAI-compatible endpoint |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` | S3 store credentials |
| `GITSTORY_WORK_UNIT` | Default work unit |
| `GITSTORY_AUTHOR` | Author of notes, links and edits (default: Git `user.name`) |
| `GITSTORY_CONFIG_DIR` | User config directory (trust list) |
| `GITSTORY_CLAUDE_PROJECTS` | Claude Code projects directory (default `~/.claude/projects`) |
| `GITSTORY_GHF` | ghf executable (default `git hf`) |

## Local state

Everything local lives under the Git common directory, never in refs or the object store:

- `.git/story/collect/<work-unit>/`: every collected event, drafts and draft egress manifests, and `meta.json` with the work unit's landings and last activity. Never shared. Drafts a landing reconciled move to `drafts/landed/<sha>/`. Every change to `meta.json` happens under `meta.lock`, a lock file held only while the meta is read, changed and written back, so a background draft and a `land` running at the same time do not undo each other's changes. A lock older than 30 seconds was left by a process that died and is taken over. The directory name is the work unit id with unsafe characters replaced by `_`, so two ids that map to the same directory (`feat/a` and `feat_a`) are refused with `E_USAGE`.
- `.git/story/store/`: the default local store.
- `.git/story/index.db`: the search index, rebuilt from the store when needed.
- `.git/story/draft.log`: output of background drafts.
