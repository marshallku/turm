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
