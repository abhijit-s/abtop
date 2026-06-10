# abtop

AI agent monitor for your terminal. Like btop++, but for AI coding agents.

Supports Claude Code, Codex CLI, and OpenCode sessions.

## Language Policy

English is mandatory for all project-facing work and communication.

- Write all source code, comments, tests, fixtures, documentation, examples, configuration text, scripts, and user-facing strings in English.
- Use English for every GitHub artifact: issue titles and bodies, issue comments, pull request titles and descriptions, review comments, commit messages, branch names, release notes, changelogs, discussions, labels, milestones, and workflow or CI messages.
- Do not use non-English text in repository content or GitHub communication unless it is an exact external identifier, a required protocol value, or a direct quote needed for context.
- When quoting or preserving non-English input, add an English explanation and keep the non-English text as short as possible.
- If a contributor opens an issue, comment, or review in another language, respond in English and continue the thread in English.

## Architecture

```
src/
├── main.rs                 # Entry, terminal setup, event loop, --setup flag
├── app.rs                  # App state, tick logic, key handling, summary generation
├── setup.rs                # StatusLine hook installation (abtop --setup)
├── ui/
│   └── mod.rs              # All panels in single file: header, context, quota,
│                           # tokens, projects, ports, sessions, footer
├── collector/
│   ├── mod.rs              # MultiCollector orchestration, orphan port detection
│   ├── claude.rs           # Claude Code: session discovery, transcript parsing
│   ├── codex.rs            # Codex CLI: session discovery via ps+lsof, JSONL parsing
│   ├── opencode.rs         # OpenCode: session discovery via ps + SQLite DB parsing
│   ├── process.rs          # Child process tree (ps) + open ports (lsof) + git stats
│   └── rate_limit.rs       # Rate limit file reading (~/.claude/abtop-rate-limits.json)
└── model/
    ├── mod.rs              # Re-exports
    └── session.rs          # AgentSession, SessionStatus, RateLimitInfo,
                            # ChildProcess, OrphanPort, SubAgent
```

## Layout

```
┌─ ¹context (token rate sparkline + per-session context bars) ─────────┐
│  ▁▃▅▇█▇▅▃▁▃▅▇██                       S1 abtop       ████████ 82%  │
│  token rate (200pt history)            S2 prediction  █████████91%⚠ │
│                                        S3 api-server  ███      22%  │
└──────────────────────────────────────────────────────────────────────┘
┌─ ²quota ─────┐┌─ ³tokens ───┐┌─ projects ───┐┌─ ⁴ports ──────────┐
│ CLAUDE       ││ Total  1.2M ││ abtop        ││ PORT  SESSION  CMD │
│ 5h ████ 35%  ││ Input  402k ││  main +3 ~18 ││ :3000 api-srv node│
│   resets 2h  ││ Output  89k ││              ││ :8080 predict crgo│
│ 7d ██ 12%    ││ Cache  710k ││ prediction   ││                    │
│              ││ ▁▃▅▇█▇▅▃▁▃▅││  feat/x +1~2 ││ ORPHAN PORTS       │
│ CODEX        ││ Turns: 48   ││              ││ :4000 old-prj node│
│ 5h █ 9%     ││ Avg: 25k/t  ││ api-server   ││                    │
│ 7d ██ 14%    ││             ││  main ✓clean ││                    │
└──────────────┘└─────────────┘└──────────────┘└────────────────────┘
┌─ ⁵sessions ─────────────────────────────────────────────────────────┐
│ ►*CC 7336 abtop  ● Work opus  82% 1.2M  48  Edit src/pay.rs       │
│  >CD 8840 pred   ◌ Wait sonn  91% 340k  12  waiting                │
│ ─────────────────────────────────────────────────────────────────── │
│  SESSION 7336 · /Users/graykode/abtop                               │
│  Stripe payment integration...                                      │
│  └─ Edit src/pay.rs                                                 │
│  CHILDREN: 7401 cargo build                                         │
│  SUBAGENTS: explore-data ✓12k · run-tests ●8k                      │
│  MEM 4f · 12/200 │ v2.1.86 · 47m                                   │
└──────────────────────────────────────────────────────────────────────┘
```

Panel rendering priority (top to bottom):
1. **Sessions** — always visible, gets priority allocation (min 5 rows, ideal = 2/session + 7)
2. **Mid-tier** (quota, tokens, projects, ports) — split equally, shown if space allows
3. **Context** — only renders when sessions have ideal height AND surplus >= 5 rows
4. **Header** (1 row) + **Footer** (1 row) — always present

Panel descriptions:
- **¹context**: Left = token rate braille sparkline (200-point history). Right = per-session context % bars with yellow/red warning.
- **²quota**: Claude + Codex rate limit gauges side-by-side (5h and 7d windows with reset countdown). Quota is intentionally limited to Claude and Codex; do not add an OpenCode row unless OpenCode exposes a reliable account-level provider rate-limit source.
- **³tokens**: Total token breakdown (in/out/cache) + per-turn sparkline for selected session.
- **projects** (always visible): Per-project git branch + added/modified file counts.
- **⁴ports**: Agent-spawned open ports + orphan ports (from dead sessions). Conflict detection.
- **⁵sessions**: Full-width panel below mid row. Session list table (top) + selected session detail (bottom), separated by divider.

## Data Sources

All read-only from local filesystem + `ps` + `lsof`. No API calls, no auth.

### 1. Claude Code session discovery: process + config-root mapping

Discovery strategy:
1. Find running `claude` processes via `ps`
2. Map PID → open files/directories via `lsof`
3. Infer Claude config roots from open paths that contain `sessions/` and `projects/`
4. Read `{config-root}/sessions/{PID}.json`, falling back to scanning session files for the matching embedded PID
5. Parse `{config-root}/projects/{encoded-path}/{sessionId}.jsonl`

Fallback config roots are still scanned: `~/.claude`, direct home profile roots matching `~/.claude-*` when they contain both `sessions/` and `projects/`, `claude_config_dirs` from `~/.config/abtop/config.toml`, abtop's own `CLAUDE_CONFIG_DIR`, and on Linux any `CLAUDE_CONFIG_DIR` read from `/proc/{pid}/environ`.

Session file format:
```json
{ "pid": 7336, "sessionId": "2f029acc-...", "cwd": "/Users/graykode/abtop", "startedAt": 1774715116826, "kind": "interactive", "entrypoint": "cli" }
```
- ~170 bytes. Created on start, deleted on exit.
- Verify PID alive with shared `ps` data containing a `claude` binary.
- Skip sessions whose PID descends from abtop's own `claude --print` summary children without hiding user-spawned non-interactive sessions.

### 2. Claude Code transcript: `{config-root}/projects/{encoded-path}/{sessionId}.jsonl`
Path encoding: `/Users/foo/bar` → `-Users-foo-bar`

Key line types:

**`assistant`** (tokens, model, tools):
```json
{
  "type": "assistant",
  "timestamp": "2026-03-28T15:25:55.123Z",
  "message": {
    "model": "claude-opus-4-6",
    "stop_reason": "end_turn",
    "usage": {
      "input_tokens": 2,
      "output_tokens": 5,
      "cache_read_input_tokens": 11313,
      "cache_creation_input_tokens": 4350
    },
    "content": [
      { "type": "text", "text": "..." },
      { "type": "tool_use", "name": "Edit", "input": { "file_path": "src/main.rs", ... } }
    ]
  }
}
```

**`user`** (prompts, version):
```json
{ "type": "user", "timestamp": "...", "version": "2.1.86", "gitBranch": "main", "message": { "role": "user", "content": "..." } }
```

**`last-prompt`** (session tail marker):
```json
{ "type": "last-prompt", "lastPrompt": "...", "sessionId": "..." }
```

- **Size: 1KB–18MB**. Append-only, new line per message.
- **Reading strategy**: On first discovery, scan full file to build cumulative token totals. Then watch file size — on growth, read only new bytes appended since last read (track file offset). This gives both lifetime totals and real-time updates without re-reading.
- **Partial line handling**: new bytes may end mid-JSON-line. Buffer incomplete lines until next read.
- **File rotation**: if file shrinks (session restart), reset offset to 0 and re-scan.

### 3. Codex CLI sessions: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`

Discovery strategy:
1. Find running `codex` processes via `ps`
2. Map PID → open `rollout-*.jsonl` file via `lsof`
3. Parse JSONL for `session_meta`, `token_count` (includes rate_limits), `agent_message` events
4. Detect finished sessions: scan today's directory for JSONL < 5 min old not owned by running process

Rate limits extracted from `token_count` events:
```json
{
  "rate_limits": {
    "limit_id": "codex",
    "primary": { "used_percent": 9.0, "window_minutes": 300, "resets_at": 1774686045 },
    "secondary": { "used_percent": 14.0, "window_minutes": 10080, "resets_at": 1775186466 },
    "plan_type": "plus"
  }
}
```

### 4. OpenCode sessions: `~/.local/share/opencode/opencode.db`
- Discover running `opencode` processes via shared `ps` data.
- Read recent sessions from OpenCode's SQLite DB through `sqlite3 -readonly -json`.
- Match live PIDs to DB sessions by process cwd. OpenCode does not expose a PID/session mapping, so when multiple DB rows share one cwd, only live PIDs should be assigned and older rows should not be shown as live duplicates.
- OpenCode contributes session/token/project/port data, but not quota data. Quota remains Claude + Codex only.

### 5. Subagents: `~/.claude/projects/{path}/{sessionId}/subagents/`
- `agent-{hash}.jsonl` — same JSONL format as main transcript
- `agent-{hash}.meta.json` — `{ "agentType": "general-purpose", "description": "..." }`

### 6. Process tree: `ps` + `lsof`
```bash
ps -eo pid,ppid,rss,%cpu,command    # All processes
lsof -i -P -n -sTCP:LISTEN         # Open ports
```
- Build parent→children map from ppid
- Map listening PID → parent agent PID → session

### 7. Git status per project
```bash
git -C {cwd} status --porcelain     # added/modified file counts
```

### 8. Memory status
- Path: `~/.claude/projects/{encoded-path}/memory/`
- Count files in directory + lines in `MEMORY.md`

### 9. Rate limit (Claude Code)

NOT in transcript JSONL. Collected via StatusLine mechanism.

`abtop --setup` automates this: creates a script at `~/.claude/abtop-statusline.sh` that writes rate limit JSON to `~/.claude/abtop-rate-limits.json`, and registers it in `~/.claude/settings.json`.

File format read by abtop:
```json
{
  "source": "claude",
  "five_hour": { "used_percentage": 35.0, "resets_at": 1774715000 },
  "seven_day": { "used_percentage": 12.0, "resets_at": 1775320000 },
  "updated_at": 1774714400
}
```
- Rejects stale data (> 10 minutes old).
- `rate_limits` only present for Pro/Max subscribers.
- Account-level metric, shared across all sessions.
- Show "—" when not configured or data unavailable.

### 10. Other files
- `~/.claude/stats-cache.json` — daily aggregates. Only updated on `/stats`, NOT real-time.
- `~/.claude/history.jsonl` — prompt history with sessionId.

## Session Status Detection

```
● Working  = PID alive + transcript mtime < 30s ago
◌ Waiting  = PID alive + transcript mtime > 30s ago
✗ Error    = PID alive + last assistant has error content
✓ Done     = PID dead (detected via kill(pid, 0) failure)
```

**Done detection**: session files are deleted on normal exit, but may linger briefly or survive crashes. When PID is dead but file exists, show as Done and clean up on next tick.

**PID reuse risk**: verify PID is still the expected agent process (Claude, Codex, or OpenCode) by checking `ps -p {pid} -o command=`. Don't trust PID alone.

Current task (2nd line under each session):
- Working → last `tool_use` name + first arg (e.g. `Edit src/main.rs`)
- Waiting → "waiting for user input"
- Error → last error message (truncated)
- Done → "finished {duration} ago"

**Known limitations** (all heuristic):
- Cannot distinguish model-thinking vs tool-executing vs rate-limit-waiting vs permission-prompt
- "Waiting" may be wrong if a long-running tool (cargo build, npm test) is running
- Status is best-effort, not authoritative

## Session Summary Generation

Each session gets a one-line summary title generated via `claude --print`:
- Spawned as background process with 10s timeout
- Rejects generic/empty output; falls back to sanitized first prompt (28 chars)
- Cached to `~/.cache/abtop/summaries.json` (persists across runs)
- Max 3 concurrent summary jobs, max 2 retries per session

## Context Window Calculation

Not provided in data files. Derive:
- **Window size**: hardcode by model name
  - `claude-opus-4-6` → 200,000 (default)
  - `claude-opus-4-6[1m]` → 1,000,000
  - `claude-sonnet-4-6` → 200,000
  - `claude-haiku-4-5` → 200,000
- **Current usage**: last `assistant` line's `input_tokens + cache_read_input_tokens`. `cache_creation_input_tokens` is intentionally excluded — on compaction turns the same tokens can be reported as both `cache_creation` *and* `cache_read`, and summing all three double-counts (#54). Matches Claude Code's own statusline and the Codex collector.
- **Percentage**: current_usage / window_size * 100
- **Warning**: yellow at 80%, red at 90%, ⚠ icon at 90%+

## Orphan Port Detection

Tracks child processes that have open ports. When a parent session dies but the child process remains alive and listening:
- Added to `orphan_ports` list automatically
- Displayed in ports panel under "ORPHAN PORTS" section
- Can be killed via `X` (Shift+X) with safety checks (fresh port scan + PID command verification before SIGKILL)

## Key Bindings

| Key | Action |
|-----|--------|
| `↑`/`↓` or `k`/`j` | Select session in list |
| `Enter` | Jump to session terminal (tmux only) |
| `x` | Kill selected session (SIGKILL) |
| `X` | Kill all orphan ports |
| `q` | Quit |
| `r` | Force refresh |

## Tech Stack

- **Rust** (2021 edition)
- **ratatui** + **crossterm** for TUI
- **serde** + **serde_json** for JSON/JSONL parsing
- **chrono** for timestamp formatting
- **dirs** for home directory resolution
- **Polling intervals** (staggered to avoid freezes):
  - Session scan + transcript tail: every 2s
  - Process tree (ps): every 2s
  - Port scan (lsof) + git status + rate limits: every 10s (5 ticks)

## Live event stream

abtop emits a pub-sub event feed alongside the TUI and `--json` snapshot. The
feed is a best-effort observable of state transitions — it complements the
snapshot for tools that want push semantics without polling.

### Transport

Unix-domain socket (UDS) by default, or TCP when `--events-tcp host:port` is
passed. Encoding is NDJSON: one `WireRecord` per line. Each record has the
shape `{ v, ts_ms, ...event_fields }` — the active `AppEvent` variant is
flattened in via `#[serde(flatten)]` and carries a `type` discriminator
string. Delivery is best-effort: per-connection queues are bounded, and when a
queue fills the publisher drops the oldest records, then emits a
`{"_meta":"dropped","count":N}` marker on the next successful write so
consumers can resync.

Socket permissions are 0600 (owner-only). Stale sockets from prior runs are
auto-removed on bind.

### Socket path resolution

| Platform                            | Default                          |
| ----------------------------------- | -------------------------------- |
| Linux (`$XDG_RUNTIME_DIR` set)      | `$XDG_RUNTIME_DIR/abtop.sock`    |
| macOS                               | `$TMPDIR/abtop.sock`             |
| Fallback                            | `/tmp/abtop-$UID.sock`           |

Override precedence (highest first): CLI `--events=<path>` > `ABTOP_EVENTS_SOCKET`
env > `[events].socket` config > platform default. `--print-socket-path`
prints the resolved value without launching the TUI. macOS/BSD `sun_path` is
capped at 104 bytes; an overlong path returns a clear error pointing at
`ABTOP_EVENTS_SOCKET`.

### Event variants

Source of truth: `src/events/types.rs`. The table below indexes the variants
in source order — for full field types and doc comments, read the file.

| `type`                | Tier | Kind       | Key fields                          |
| --------------------- | ---- | ---------- | ----------------------------------- |
| `SessionStarted`      | Fast | Lifecycle  | `session_id, agent_cli, pid`        |
| `SessionEnded`        | Fast | Lifecycle  | `session_id`                        |
| `StatusChanged`       | Fast | Status     | `session_id, from, to`              |
| `ContextThreshold`    | Fast | Threshold  | `session_id, bucket`                |
| `TokenRateSpike`      | Fast | Threshold  | `rate, baseline`                    |
| `RateLimited`         | Slow | Quota      | `provider, resets_at_ms`            |
| `RateLimitCleared`    | Slow | Quota      | `provider`                          |
| `OrphanPortAppeared`  | Slow | Resource   | `port, prior_pid`                   |
| `McpServerAppeared`   | Slow | Resource   | `pid, parent_cli`                   |
| `McpServerVanished`   | Slow | Resource   | `pid`                               |
| `ToolCalled`          | Fast | Activity   | `session_id, tool, arg`             |
| `CompactionDetected`  | Fast | Activity   | `session_id`                        |
| `SubagentSpawned`     | Fast | Activity   | `session_id, name`                  |
| `HostLoadHigh`        | Fast | Host       | `load1`                             |

Fast events are evaluated every tick (~2s). Slow events only emit on ticks
where the slow collector branch (lsof, rate limits) ran (~10s).

`ContextThreshold` buckets are 70/90/95 with hysteresis — each bucket is an
independent latch with a separate clear threshold, so flapping near a
boundary won't spam.

### Schema versioning

The wire schema version is exposed as the constant `events::types::WIRE_VERSION`
and is currently `1`. Additive fields (new event variants, new optional
fields on existing variants) do NOT bump `v`. Renames, removals, or type
changes DO. Consumers that care about forward-compat should branch on `v`.

### Config schema

Main file: `$XDG_CONFIG_HOME/abtop/config.toml` (or `$HOME/.config/abtop/config.toml`).
Drop-ins from `$XDG_CONFIG_HOME/abtop/plugins.d/*.toml` are merged in
filename order. Precedence: built-in defaults → main file → drop-ins (rule
arrays append, scalars override) → `ABTOP_EVENTS_SOCKET` env → CLI flags.

```toml
# ~/.config/abtop/config.toml

[events]
enabled = true                                  # off by default; --events also enables
socket  = "${XDG_RUNTIME_DIR}/abtop.sock"       # ${UID}, ${XDG_RUNTIME_DIR}, and ~ expand
backlog = 256                                   # per-connection NDJSON queue depth

[plugins.notifier]
enabled      = true                             # start notifier worker at launch
backend      = "auto"                           # auto | osascript | notify-send | terminal-notifier | stderr
debounce_ms  = 5000                             # default per-rule debounce window

# Rule grammar:
#   on          = list of event `type` names; empty / omitted = wildcard
#   when        = single comparison, "<field> <op> <literal>"
#                 op      ∈ { ==, !=, <=, >=, <, > }
#                 literal = "quoted string" OR bare number
#                 (no `and`/`or` — write two rules instead)
#   title/body  = templates; `{field}` substitutes from the event,
#                 plus `{kind}`, `{ts_ms}`, `{v}`
#   debounce_ms = optional per-rule override of the table-level default

[[plugins.notifier.rule]]
on    = ["ContextThreshold"]
when  = "bucket >= 90"
title = "{session_id} context {bucket}%"
body  = "Approaching window limit at {ts_ms}"

[[plugins.notifier.rule]]
on          = ["RateLimited"]
when        = 'provider == "claude"'
title       = "Claude rate-limited"
body        = "Resets at {resets_at_ms}"
debounce_ms = 60000
```

Invalid `when` expressions disable only the offending rule and log a single
stderr line at load — they never take the worker down.

### Hot reload

Config files are mtime-polled by `App::tick`. Changes apply live for:

- `[events].enabled` — pauses or resumes the publisher in place
- `[plugins.notifier].enabled` — pauses or resumes the notifier worker
- `[plugins.notifier].rule` — recompiled on each change
- `[plugins.notifier].debounce_ms` — applied to subsequent fires
- `[plugins.notifier].backend` — re-probes the chosen backend
- `[plugins.system_notifier].enabled` — pauses or resumes the worker
- `[plugins.system_notifier].conduit` / `conduit_args` — applied to the
  next invocation
- `[plugins.system_notifier].on` / `title` / `body` / `debounce_ms` /
  `conduit_timeout_ms` — applied to subsequent fires

Changes require restart for:

- `[events].socket` — the listener is bound once at startup
- `[events].backlog` — queue depth is fixed per connection

When a restart-required field changes, abtop logs `restart required to apply
<field>` and keeps running with the previous value.

### CLI flags

- `--events[=path]` — enable the publisher, optionally overriding the socket path
- `--events-off` — force-disable the publisher even if config enables it
- `--events-tcp host:port` — bind a TCP listener instead of a UDS
- `--print-socket-path` — print the resolved socket path and exit
- `--plugin-notify` / `--no-plugin-notify` — force the notifier on/off
- `--plugin-system-notify` / `--no-plugin-system-notify` — force the
  System Notifier on/off (same precedence as the notifier flags)
- `--config <path>` — use a specific config file
- `--no-config` — skip config files entirely (defaults + flags only)
- `--list-plugins` — print the catalogue of plugins compiled into this
  binary, their default startup state, and a copy-pasteable config
  snippet for `~/.config/abtop/config.toml`

### TUI controls

- `e` — toggle pause/resume on the publisher; footer shows current state
- `v` — view menu; the Events row shows the full untruncated socket path,
  live connection count, and lifetime dropped-record count

### Notifier backends

| Backend             | Platform     | Notes                                     |
| ------------------- | ------------ | ----------------------------------------- |
| `osascript`         | macOS        | Built-in; no extra install                |
| `terminal-notifier` | macOS        | Optional `brew install terminal-notifier` |
| `notify-send`       | Linux        | From `libnotify-bin` or distro equivalent |
| `stderr`            | Any          | Always-available fallback                 |

Auto-probe order:

- macOS: `osascript` → `terminal-notifier` → `stderr`
- Linux: `notify-send` → `stderr`
- Other Unix: `stderr` only

`backend = "auto"` (the default) walks the platform list and picks the first
binary that responds to a cheap `--version` / `-help` probe. Explicit values
skip the probe and fail closed to `stderr` if the named backend is
unavailable.

### System Notifier

A second compiled-in plugin that surfaces events through a user-supplied
conduit binary. Unlike the built-in `notifier` (which dispatches to
osascript / notify-send / terminal-notifier), the System Notifier owns the
connect-loop and NDJSON parsing but delegates the actual notification to a
script the user writes — `notify.sh`, an `ntfy` CLI wrapper, a curl
webhook, anything that reads stdin and reacts to env vars. One conduit per
plugin instance; routing across multiple notification surfaces belongs in
the conduit script.

```toml
[plugins.system_notifier]
enabled            = true                                       # off by default
conduit            = "~/bin/notify.sh"                           # ~ and ${VAR} expand
conduit_args       = []                                          # extra positional args
on                 = ["RateLimited", "ContextThreshold"]         # empty = all event types
title              = "abtop: {kind}"                             # rendered before spawn
body               = "{session_id}"                              # same {field} grammar as notifier
debounce_ms        = 5000                                        # per (rule, event-identity) window
conduit_timeout_ms = 5000                                        # per-invocation wall-clock cap
```

Conduit contract: each event spawns the conduit fresh. The full
`WireRecord` JSON is written to stdin and stdin is closed; the rendered
title/body and curated metadata land in env vars before exec:

- `ABTOP_TITLE` — rendered `title` template (post-`{field}` substitution)
- `ABTOP_BODY` — rendered `body` template
- `ABTOP_EVENT_TYPE` — the `AppEvent` variant name (e.g. `RateLimited`)
- `ABTOP_TS_MS` — event timestamp in milliseconds since the Unix epoch
- `ABTOP_FIELD_<KEY>` — one var per flat string/number top-level field on
  the event, hard-capped at 4 KiB each (the full payload is still on
  stdin, so truncation here only affects the convenience shortcut)

Failure semantics are log + drop. The reader thread `try_send`s into a
bounded `mpsc::sync_channel(32)`; on full the event is dropped and a
rate-limited stderr line surfaces the count. The invocation thread
enforces `conduit_timeout_ms` with a kill-and-reap; non-zero exits and
spawn failures are logged once per 30s and never retried — wrap your
script in a circuit breaker if you need that semantics.

The System Notifier appears in `abtop --list-plugins` alongside the
notifier; its catalogue entry includes a copy-pasteable snippet matching
the block above.

### Discovering plugins

Run `abtop --list-plugins` to see which plugins are compiled into your
binary, their default startup state, and a copy-pasteable config snippet
for `~/.config/abtop/config.toml`. With the default feature set this
lists both the `notifier` and `system_notifier`; with
`--no-default-features` the catalogue is empty and the command tells you
how to rebuild with a plugin feature.

## Commit Convention

```
<type>: <description>
```
Types: `feat`, `fix`, `refactor`, `docs`, `chore`

## Commands

```bash
cargo build                    # Build
cargo run                      # Run TUI
cargo run -- --once            # Print snapshot and exit
cargo run -- --setup           # Install StatusLine hook for rate limit collection
cargo run -- --exit-on-jump    # Quit after Enter-jumping to a tmux pane (for popup overlays)
cargo test                     # Tests
cargo clippy                   # Lint
```

## Release Process

1. Pick the target semver version and update both `Cargo.toml` and `Cargo.lock`.
2. Verify the package locally:
   ```bash
   cargo test
   cargo clippy -- -D warnings
   cargo build --release
   cargo publish --dry-run
   ```
3. Commit and merge or push the version bump to `main`:
   ```bash
   git add Cargo.toml Cargo.lock
   git commit -m "chore: bump version to X.Y.Z"
   git push origin main
   ```
4. From a clean, up-to-date `main`, create and push an annotated release tag:
   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin vX.Y.Z
   ```
5. Watch the tag-triggered workflows:
   ```bash
   gh run list --workflow Release --limit 5
   gh run list --workflow "Publish to crates.io" --limit 5
   ```
6. `release.yml` builds platform binaries, creates the GitHub Release, and updates the Homebrew formula.
7. `publish.yml` runs `cargo publish` to crates.io automatically.

**Do NOT run `cargo publish` or `gh release create` manually** — the CI workflows handle both.
**Do NOT push the tag before the version bump is on `main`.**
**Do NOT reuse a release tag after a failed publish; bump to a new patch version instead.**

## Non-Goals (v0.1)

- Gemini/Cursor support
- Cost estimation
- Remote/SSH monitoring
- Notifications/alerts

## tmux Integration

Session jump (`Enter`) only works when abtop runs inside tmux:
1. On startup, detect if `$TMUX` is set. If not, disable Enter key.
2. To map PID → tmux pane: `tmux list-panes -a -F '#{pane_pid} #{session_name}:#{window_index}.#{pane_index}'` then walk process tree to find which pane owns the agent PID.
3. Jump: `tmux select-pane -t {target}`
4. If mapping fails (PID not in any pane), show transient "pane not found" status message.

## Privacy

abtop reads transcripts, prompts, tool inputs, and memory files. These may contain secrets.
- **`--once` output**: redact file contents from tool_use inputs. Show tool name + file path only, not content.
- **TUI mode**: show tool name + first arg (file path), never show file contents or prompt text in session list.
- **No network**: abtop never sends data anywhere. All local reads.
- **Exception**: summary generation calls `claude --print` locally (no network by abtop itself, but claude may use its API).

## Gotchas

- **Transcript size**: 1KB–18MB. On first load, full scan for totals. After that, track file offset and read only new bytes. Buffer partial lines.
- **Session file deletion**: files disappear when Claude exits. Handle `NotFound` between scan and read.
- **stats-cache.json is stale**: only updated on `/stats` command. Don't use for live data.
- **Context window not in data**: must hardcode per model. Will break if Anthropic/OpenAI add new models.
- **Rate limit is account-level**: shared across all sessions. Don't show per-session.
- **Path encoding**: `/Users/foo/bar` → `-Users-foo-bar`. Used for transcript directory names.
- **Path encoding collision**: `-Users-foo-bar-baz` could be `/Users/foo/bar-baz` or `/Users/foo-bar/baz`. Use session JSON's `cwd` as source of truth.
- **lsof can be slow**: on macOS with many open files. Cache results, poll every 10s.
- **Child process tree**: `pgrep -P` only gets direct children. Build full tree from `ps -eo ppid`.
- **Port detection race**: a port can close between lsof and display. Show stale data gracefully.
- **Subagent directory may not exist**: only created when Agent tool is used. Check existence before scanning.
- **Undocumented internals**: all data sources are Claude Code/Codex implementation details, not stable APIs. Schema may change without notice. Defensive parsing with `serde(default)` everywhere.
- **Terminal size**: minimum 80x24. Panels degrade gracefully when small (context panel hidden first).
- **PID reuse in port cache**: invalidate cached ports when the set of tracked PIDs changes.
- **Rate limit staleness**: reject rate limit data older than 10 minutes.
- **`/clear` + multi-PID same cwd**: after `/clear`, Claude Code mints a new `sessionId` + `.jsonl` without rewriting `sessions/{PID}.json`. abtop overrides the stale sid by picking the newest transcript in the project dir, but this heuristic can't disambiguate ownership when two live `claude` PIDs share a cwd — so the override is disabled in that case and both sessions keep their original sid until exit. Use separate worktrees if live tracking is needed on both simultaneously.
