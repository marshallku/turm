# Persistence & Agent Resume

comux persists your whole workspace and restores it when the server restarts — sessions, tabs, split layout, and each pane's working directory. For agent panes, it goes further: it brings the agent back **mid-conversation**. The model follows tmux-resurrect / tmux-continuum.

## How persistence works

- The layout — sessions, tabs, splits, per-pane cwd — is **autosaved** every `autosave_secs` (default 15s) and on server shutdown, written by an off-loop writer thread using a temp + fsync + rename + dir-fsync sequence so the file is never left half-written.
- On server start, if `persist = true` (the default), the layout is **restored**. Shells restart fresh **in their saved directory**.
- The save file lives at `$COPAD_MUX_STATE`, else `~/.local/state/copad/mux-session.json`.

The semantics are "continuum-style": comux always restores the last layout. To start fresh, set `persist = false` or delete the state file.

```bash
# start clean once, without touching config:
rm ~/.local/state/copad/mux-session.json
comux
```

A malformed or newer-schema state file is ignored and never blocks startup.

---

## Re-running programs on restore

By default a restored pane comes back as a bare shell in its saved directory — running programs are **not** relaunched. The exception is **whitelisted programs**: if a pane's foreground command's basename is in `restore_processes`, its full command line is saved and re-injected into the fresh shell.

`restore_processes` defaults to the AI agents:

```toml
restore_processes = ["claude", "codex", "aider", "cursor", "gemini",
                     "opencode", "droid", "copilot", "qwen", "crush"]
```

So out of the box, your agents relaunch on restart and everything else restores as a plain shell. To restore bare shells only, set `restore_processes = []`. To also bring back, say, an editor, add its basename:

```toml
restore_processes = ["claude", "codex", "nvim"]
```

Restored panes start at the scrollback bottom — history is not saved.

---

## Agents resume their live conversation

This is the payoff. With `restore_agent_sessions = true` (the default), a restored agent doesn't re-run its raw command line (which would start a *fresh* chat). Instead comux rebuilds the command to **resume the exact conversation the process was in**:

- **Claude** → `claude --resume <id>`, where the id comes from `~/.claude/sessions/<pid>.json`
- **Codex** → `codex resume <id>`, where the id comes from the rollout file the pid held open

The reconstruction keeps only known-safe flags (e.g. `--dangerously-skip-permissions`, `--model`) and drops the prompt and selectors. It skips one-shot invocations (`-p` / `--no-session-persistence`) and explicit codex subcommands, and guards against pid reuse with an `argv[0]`-basename match.

Set `restore_agent_sessions = false` to always restart agents clean instead.

---

## Finding a conversation you closed (`Ctrl-b R`)

Persistence covers panes that were *open* when the server stopped. `Ctrl-b R` covers the other case: a conversation whose pane you closed days ago and whose id you no longer have.

Both CLIs keep every transcript on disk, so comux lists them directly — `~/.claude/projects/<cwd>/<id>.jsonl` and `~/.codex/sessions/<date>/rollout-<ts>-<id>.jsonl`:

```
┌─ resume  (interactive · ^A all · ^R rescan) ────────────────────────────┐
│ > copad█                                                      26 found │
│ ▸ ▪ claude comux에 작업 하나 해보자. 지금 claude…      ~/dev/copad · now │
│   ▪ claude 이거 지금 깃허브에 이슈 올라온 거 있어?   ~/dev/copad · 24m │
│   ▪ codex  Pressure-test this plan before…            ~/dev/copad · 2d │
└─────────────────────────────────────────────────────────────────────────┘
```

- **Newest first**, labelled with the prompt that opened the conversation, its directory and its age.
- **Type to filter** — the query matches the prompt, the path and the tool together. Rows containing it verbatim come first, then fzf-style scattered matches.
- **`Ctrl-a` toggles scope.** By default only conversations you drove *interactively* are listed. Headless runs (`claude -p` / the SDK, `codex exec`, agents driving agents) are hidden — on a machine that automates a lot they outnumber real sessions 25:1. `Ctrl-a` shows them too (the first toggle takes a moment: their titles are read on demand).
- **`Enter` opens it in a new tab**, running `claude --resume <id>` / `codex resume <id>` in the conversation's own directory. If a space is already working in that directory, comux switches to it first; otherwise the tab lands in the current space. If the directory is gone (a deleted worktree), it opens in the current pane's directory and says so.
- **A conversation that is already open** in one of this server's panes is marked `●`, and `Enter` jumps to that pane instead of starting a second copy of it.

The scan runs on its own thread — the mux never blocks on it — and is reused for a few seconds, so reopening the picker is instant. `Ctrl-r` forces a fresh read.

---

## Restarting the server

`comux server restart` is the everyday way to pick up a new binary or clear render drift without losing anything:

```bash
comux server restart
```

It does a final save, kills the server, and spawns a fresh one that restores the persisted layout — so your sessions come back and agents resume their conversations. Attached clients drop; re-run `comux` to reattach. `stop` and `restart` are idempotent (a not-running or mid-exit server counts as already-stopped).

---

## What's fixed at boot vs. live-reloadable

Most settings apply live with `comux reload`, but three persistence-related ones are read only at server start and need a full `comux server restart`:

- `persist`
- `autosave_secs`
- `update_environment` (the environment-scrub list — see [Environment Variables](./environment.md))

`restore_processes` and `restore_agent_sessions` are read at **save time**, so a `comux reload` does take effect for them on the next save.
