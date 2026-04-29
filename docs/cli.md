# CLI Tool (turm-cli)

Binary name: `turmctl`

## Usage

```bash
turmctl [--socket <path>] [--json] <command>
```

- `--socket` — override socket path (default: `$TURM_SOCKET` or `/tmp/turm-{PID}.sock`)
- `--json` — output in JSON format

## Commands

### System

- `turmctl ping` — ping running instance

### Session

- `turmctl session list` — list all panels across all tabs
- `turmctl session info <id>` — detailed info for a panel

### Background

- `turmctl background set <path>` — set background image (path is canonicalized)
- `turmctl background clear` — clear background image
- `turmctl background set-tint <opacity>` — set tint opacity (0.0–1.0)
- `turmctl background next` — switch to next random background
- `turmctl background toggle` — toggle background visibility

### Tab

- `turmctl tab new` — create a new tab
- `turmctl tab close` — close the focused tab/panel
- `turmctl tab list` — list tabs
- `turmctl tab info` — extended tab info with panel counts
- `turmctl tab toggle-bar` — toggle tab bar collapsed/expanded
- `turmctl tab rename --id <id> <title>` — rename a tab by panel ID

### Split

- `turmctl split horizontal` — split focused pane horizontally
- `turmctl split vertical` — split focused pane vertically

### Event Stream

- `turmctl event subscribe` — subscribe to terminal events (streams JSON lines to stdout)

### WebView

- `turmctl webview open <url> [--mode tab|split_h|split_v]` — open URL in new webview panel
- `turmctl webview navigate --id <id> <url>` — navigate existing webview
- `turmctl webview back --id <id>` — go back in history
- `turmctl webview forward --id <id>` — go forward in history
- `turmctl webview reload --id <id>` — reload page
- `turmctl webview exec-js --id <id> <code>` — execute JavaScript, return result
- `turmctl webview get-content --id <id> [--format text|html]` — get page content
- `turmctl webview screenshot --id <id> [--path <file>]` — screenshot (base64 PNG or save to file)
- `turmctl webview query --id <id> <selector>` — query single DOM element
- `turmctl webview query-all --id <id> <selector> [--limit 50]` — query all matching elements
- `turmctl webview get-styles --id <id> <selector> <properties>` — get computed CSS styles
- `turmctl webview click --id <id> <selector>` — click a DOM element
- `turmctl webview fill --id <id> <selector> <value>` — type text into an input
- `turmctl webview scroll --id <id> [--selector <sel>] [--x 0] [--y 0]` — scroll to position or element
- `turmctl webview page-info --id <id>` — get page metadata (title, dimensions, element counts)
- `turmctl webview devtools --id <id> [action]` — toggle DevTools inspector (show/close/attach/detach)

### Terminal

- `turmctl terminal read [--id <id>] [--start-row N --end-row N ...]` — read visible screen text (or range)
- `turmctl terminal state [--id <id>]` — get terminal state (cursor, dimensions, CWD, title)
- `turmctl terminal exec [--id <id>] <command>` — execute command (sends text + newline)
- `turmctl terminal feed [--id <id>] <text>` — send raw text to terminal (no newline)
- `turmctl terminal history [--id <id>] [--lines 100]` — read scrollback history
- `turmctl terminal context [--id <id>] [--history-lines 50]` — get combined context (state + screen + scrollback)

### Agent

- `turmctl agent approve <message> [--title <title>] [--actions "Approve,Deny"]` — show approval dialog, block until user responds

### Plugin

- `turmctl plugin list` — list installed plugins with panels and commands
- `turmctl plugin open <plugin> [--panel main]` — open a plugin panel in a new tab
- `turmctl plugin run <plugin>.<command> [--params '{}']` — run a plugin shell command

### Todo (Phase 19.1a)

Ergonomic wrapper over the `todo.*` action surface. Every subcommand is sugar over `turmctl call todo.<name> --params '...'`; no new IPC. Use `--json` for raw payloads (scriptable), default mode for human-readable rendering.

- `turmctl todo create <title> [--body <text>] [--workspace <ws>] [--priority <low|normal|high>] [--due <iso>] [--linked-jira <KEY>] [--tags <a,b,c>]` — wraps `todo.create`. Workspace defaults to `TURM_TODO_DEFAULT_WORKSPACE` env var, else the plugin's own default.
- `turmctl todo list [--status <open|in_progress|done|blocked>] [--workspace <ws>] [--tag <name>] [--due-before <iso>] [--hide-done]` — wraps `todo.list`. Default render: `[icon] <id>  <priority>  <title>  ·  ws=<...> tags=<...>`. Status icons: `[ ]` open, `[~]` in_progress, `[x]` done, `[!]` blocked.
- `turmctl todo set <id> --status <s> [--workspace <ws>]` — wraps `todo.set_status`. Status must be `open|in_progress|done|blocked`. `--workspace` scopes id resolution when the same id exists in multiple workspaces.
- `turmctl todo done <id> [--workspace <ws>]` / `doing <id> [...]` / `block <id> [...]` — shorthands for `set --status done|in_progress|blocked`.
- `turmctl todo start <id> [--workspace <ws>]` — wraps `todo.start` (publishes `todo.start_requested` for the vision-flow-3 chain).
- `turmctl todo delete <id> [--workspace <ws>]` — wraps `todo.delete`.

**ID prefix matching**: every `<id>` argument accepts a unique prefix. The CLI preflights `todo.list` to find candidates and resolves the workspace alongside, so a todo in a non-default workspace works without the user passing `--workspace`. Todo ids are workspace-scoped (not globally unique) — if the same id exists in multiple workspaces, the CLI errors out with the candidate list and the user disambiguates via `--workspace <ws>` (or a longer prefix). Exact-id collisions are NOT silently resolved.

### Theme

- `turmctl theme list` — list available themes and current theme

### Update

- `turmctl update check` — check for newer version
- `turmctl update apply [--version <tag>]` — download and install latest version

## Protocol

Uses cmux V2 newline-delimited JSON over Unix domain socket.

Request:

```json
{ "id": "<uuid>", "method": "background.next", "params": {} }
```

Response:

```json
{"id": "<uuid>", "ok": true, "result": {...}}
```

## Socket Client (`client.rs`)

- Connects to Unix socket
- 15s read timeout, 5s write timeout
- Sends JSON request, reads matching response by ID
- Matches request/response by UUID
