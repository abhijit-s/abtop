# abtop

**Like [btop](https://github.com/aristocratos/btop), but for your AI coding agents.**

See every Claude Code, Codex CLI, and OpenCode session at a glance — token usage, context window %, rate limits, child processes, open ports, and more.
Claude Code, Codex CLI, and OpenCode sessions are discovered from local process/file state, so multiple active profiles are supported across macOS, Linux, and Windows.

![demo](https://raw.githubusercontent.com/graykode/abtop/main/assets/demo.gif)

## Why

- Running 3+ agents across projects? See them all in one screen.
- Hitting rate limits? Watch your quota in real-time.
- Agent spawned a server and forgot to kill it? Orphan port detection.
- Context window filling up? Per-session % bars with warnings.

All read-only. No API keys. No auth.

## Install

### macOS / Linux

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/graykode/abtop/releases/latest/download/abtop-installer.sh | sh
```

### Cargo

```bash
cargo install abtop
```

### Windows

Native support — no WSL required. Uses `sysinfo` for process info and `netstat -ano` for listening ports.

```powershell
powershell -c "irm https://github.com/graykode/abtop/releases/latest/download/abtop-installer.ps1 | iex"
```

Or `cargo install abtop` from any terminal with Git in PATH. Claude Code config is resolved automatically from `%USERPROFILE%\.claude`.

### Other

Pre-built binaries for all platforms are available on the [GitHub Releases](https://github.com/graykode/abtop/releases) page.

## Usage

```bash
abtop                    # Launch TUI
abtop --once             # Print snapshot and exit
abtop --json             # Print one JSON snapshot and exit (for scripts/tools)
abtop --setup            # Install rate limit collection hook
abtop --theme dracula    # Launch with a specific theme
abtop --theme /tmp/x.theme   # Or load a theme file directly (one-shot, not saved)
```

Recommended terminal size: **120x40** or larger. Minimum 80x24 — panels hide gracefully when small.

### tmux

abtop works standalone, but running inside tmux unlocks session jumping — press `Enter` to switch directly to the pane running that agent.

```bash
tmux new -s work
# pane 0: abtop
# pane 1: claude (project A)
# pane 2: claude (project B)
# → Enter on a session in abtop jumps to its pane
```

## Theming

abtop ships with 13 embedded themes. Pick one with `--theme <name>` or set
`theme = "<name>"` in `$XDG_CONFIG_HOME/abtop/config.toml` (default
`~/.config/abtop/config.toml`).

Available: `btop`, `dracula`, `catppuccin`, `catppuccin-transparent`,
`tokyo-night`, `gruvbox`, `nord`, `light`, `white`, `high-contrast`,
`protanopia`, `deuteranopia`, `tritanopia`.

`catppuccin-transparent` is identical to `catppuccin` but with `main_bg`
empty so the terminal background shows through — equivalent to setting
`theme_background = false` (below) without editing config.

You can also pass a direct file path to `--theme` instead of a name:

```sh
abtop --theme /tmp/scratch.theme
abtop --theme ./relative.theme
abtop --theme ~/foo.theme
```

`--theme <path>` is detected when the argument contains `/`, `\`, or starts
with `~`. Path-loaded themes are one-shot — `config.toml` is not modified.

### Custom themes

Drop a `*.theme` file into `$XDG_CONFIG_HOME/abtop/themes/` and reference
it by file basename. The format is btop-compatible:

```sh
# ~/.config/abtop/themes/my-theme.theme
theme[main_bg]="#1e1e2e"      # 6-digit hex, or empty for terminal default
theme[main_fg]="#cdd6f4"
theme[selected_bg]="#313244"
# ... see themes/btop.theme in the source tree for the full key list
```

Missing keys inherit from the embedded `btop` theme. Empty values on any
`Color` field render as `Color::Reset` (the terminal's own default), which
on terminals with transparency configured lets the background show through.
Empty values on the `*_grad_*` gradient channels fall back to btop's
gradient instead — gradients are RGB tuples and have no terminal-default
representation.

If your file contains malformed lines (bad hex, unknown keys, missing
quotes), abtop shows a 3-second footer banner at launch with the error
count. The theme still loads — the broken fields fall back to btop
defaults — so you can iterate without abtop becoming unusable.

Edits to the active `.theme` file are picked up automatically on the
next tick (~2 seconds) — no need to restart abtop. The footer briefly
shows `theme '<name>' reloaded` (or `… with N parse errors` if any).

User-dir files override embedded themes of the same name. Custom themes also
join the `t`-key cycle at startup; mid-session additions require a restart.

### Discovering and editing themes

List all themes available right now — embedded plus anything you've dropped
into `$XDG_CONFIG_HOME/abtop/themes/`:

```sh
$ abtop --list-themes
btop (built-in)
dracula (built-in)
catppuccin (built-in)
catppuccin-transparent (built-in)
tokyo-night (built-in)
gruvbox (built-in)
nord (built-in)
light (built-in)
white (built-in)
high-contrast (built-in)
protanopia (built-in)
deuteranopia (built-in)
tritanopia (built-in)
```

If a user file shadows an embedded theme of the same name, the embedded
entry is marked `(user override)`. User-only themes (no embedded
counterpart) are appended at the bottom and marked `(user)`.

To edit one of the built-in themes, dump its body to the user dir first:

```sh
$ abtop --dump-theme catppuccin
wrote /home/me/.config/abtop/themes/catppuccin.theme

$ $EDITOR ~/.config/abtop/themes/catppuccin.theme
# tweak away; user file now overrides the embedded version
```

`--dump-theme` refuses to overwrite an existing file. Pass `--force` to
overwrite. Only embedded themes can be dumped (user-only themes are
already on disk).

### Transparent background

Add `theme_background = false` to your `config.toml` to force `main_bg` to
the terminal default for any theme — no need to edit the theme file:

```toml
theme = "catppuccin"
theme_background = false
```

`selected_bg` and `meter_bg` keep their theme values (they're visible-state
indicators, not the window background).

## Supported Agents

| Feature           | Claude Code | Codex CLI | OpenCode |
| ----------------- | :---------: | :-------: | :------: |
| Session Discovery |     ✅      |    ✅     |    ✅    |
| Token Tracking    |     ✅      |    ✅     |    ✅    |
| Context Window %  |     ✅      |    ✅     |    ❌    |
| Status Detection  |     ✅      |    ✅     |    ✅    |
| Current Task      |     ✅      |    ✅     |    ❌    |
| Rate Limit        |     ✅      |    ✅     |    ❌    |
| Git Status        |     ✅      |    ✅     |    ✅    |
| Children / Ports  |     ✅      |    ✅     |    ✅    |
| Subagents         |     ✅      |    ❌     |    ❌    |
| Memory Status     |     ✅      |    ❌     |    ❌    |

OpenCode support reads the local SQLite database at `~/.local/share/opencode/opencode.db` and requires `sqlite3` in `PATH`.

## Themes

13 built-in themes, including a transparent variant (`catppuccin-transparent`) and 4 colorblind-friendly options (`high-contrast`, `protanopia`, `deuteranopia`, `tritanopia`). Press `t` to cycle at runtime, or launch with `--theme <name>`. Your choice is saved to `~/.config/abtop/config.toml`. See the [Theming](#theming) section above for the full list and custom-theme format.

| btop (default) | dracula | catppuccin |
|:-:|:-:|:-:|
| ![btop](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/btop.png) | ![dracula](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/dracula.png) | ![catppuccin](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/catppuccin.png) |

| tokyo-night | gruvbox | nord |
|:-:|:-:|:-:|
| ![tokyo-night](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/tokyo-night.png) | ![gruvbox](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/gruvbox.png) | ![nord](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/nord.png) |

Colorblind-friendly themes:

| high-contrast | protanopia |
|:-:|:-:|
| ![high-contrast](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/high-contrast.png) | ![protanopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/protanopia.png) |

| deuteranopia | tritanopia |
|:-:|:-:|
| ![deuteranopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/deuteranopia.png) | ![tritanopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/tritanopia.png) |

Light themes (`light` — Solarized cream, `white` — GitHub-style pure white) for bright terminals:

| light | white |
|:-:|:-:|
| ![light](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/light.png) | ![white](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/white.png) |

## Configuration

`~/.config/abtop/config.toml` supports:

```toml
theme = "btop"
# Hide specific agent CLIs from the TUI (case-insensitive).
# Useful if you only use one agent and want a cleaner view.
hidden_agents = ["codex"]
# Additional Claude Code profile roots to scan.
# abtop also auto-discovers ~/.claude and ~/.claude-* roots that contain
# both sessions/ and projects/.
claude_config_dirs = ["~/.claude-personal", "~/.claude-work-team"]
# UI language. Omit or leave empty to auto-detect from LANG.
language = "zh"
```

### Supported Languages

| Code | Language            |
| ---- | ------------------- |
| `en` | English (default)   |
| `zh` | Simplified Chinese  |

When `language` is unset, abtop auto-detects from `LANG` — any value starting with `zh` switches to Simplified Chinese, otherwise English.

## Key Bindings

| Key                | Action                               |
| ------------------ | ------------------------------------ |
| `↑`/`↓` or `k`/`j` | Select session                       |
| `Enter`            | Jump to session terminal (tmux only) |
| `x`                | Kill selected session                |
| `X`                | Kill all orphan ports                |
| `t`                | Cycle theme                          |
| `1`–`5`            | Toggle panel visibility              |
| `Esc`              | Open/close config page               |
| `q`                | Quit                                 |
| `r`                | Force refresh                        |

## Library / JSON snapshot

abtop is also a library crate, so local tools can reuse its data-collection
layer in-process — no re-scanning, no subprocesses — and serialize the same
state the TUI renders.

```bash
abtop --json    # one-shot JSON snapshot for scripts
```

For long-running consumers, build an `App`, refresh it with
`App::tick_no_summaries()` (which never spawns `claude --print`, so it doesn't
touch your Claude quota), and call `App::to_snapshot(interval_ms)` to get a
JSON-serializable [`Snapshot`]:

```rust,no_run
use abtop::app::App;
use abtop::{config, theme::Theme};

let cfg = config::load_config();
let mut app = App::new_with_config_and_claude_dirs(
    Theme::default(), &cfg.hidden_agents, cfg.panels, &cfg.claude_config_dirs,
);
app.tick_no_summaries();
let json = serde_json::to_string(&app.to_snapshot(2_000)).unwrap();
```

`App` is not `Send` (it owns the collectors), so keep it on one thread and pass
the serialized JSON elsewhere. [abtop-web-ui](https://github.com/XKHoshizora/abtop-web-ui)
is a reference consumer: a local-first web dashboard built on exactly this API.

## Privacy

abtop reads local files and local process/open-file metadata only. No API keys, no auth. In the TUI and `--once` output, tool names and file paths are shown, but file contents and prompt text are never displayed. Session summaries are generated via `claude --print`, which makes its own API call — this is the only indirect network usage.

The JSON snapshot includes richer local dashboard data, including `summary`, `chat_messages`, working directories, config roots, tool-call previews, child process commands, token counts, and port metadata. Chat text is bounded and redacted by the collectors, but it is still derived from local transcripts and may contain sensitive project context. Treat JSON snapshots as local/private data and avoid writing them to shared logs or exposing them on a network without your own access controls.

## Acknowledgements

Huge thanks to [@tbouquet](https://github.com/tbouquet) for driving much of abtop's recent shape — themes, config overlay and panel toggles, session filtering, subagent tree view, the context window gauge with compaction detection, plus a steady stream of fixes and security hardening along the way.

## License

MIT
