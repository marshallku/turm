# Core Library (turm-core)

Shared Rust library used by all platform targets.

## Modules

### config.rs

TOML config at `~/.config/turm/config.toml`.

```rust
TurmConfig {
    terminal: TerminalConfig { shell, font_family, font_size },
    background: BackgroundConfig { directory, interval, tint, opacity },
    socket: SocketConfig { path },
    theme: ThemeConfig { name },
}
```

Key methods:

- `TurmConfig::load()` — reads config file, returns defaults if missing
- `TurmConfig::write_default()` — creates default config file
- `TurmConfig::config_path()` — returns `~/.config/turm/config.toml`

Defaults:

- shell: `$SHELL` or `/bin/sh`
- font: JetBrainsMono Nerd Font Mono, size 14
- tint: 0.9, opacity: 0.95
- socket: `/tmp/turm.sock`
- theme: `catppuccin-mocha`

### background.rs

Background image cache manager.

```rust
BackgroundManager {
    directory: Option<PathBuf>,
    cache_file: PathBuf,        // ~/.cache/turm/wallpapers.txt
    current: Option<PathBuf>,
    cached_images: Vec<PathBuf>,
}
```

Key methods:

- `load_cache()` — reads cache file, rebuilds if empty or missing
- `rebuild_cache()` — scans directory for image files (jpg, jpeg, png, webp, bmp)
- `next()` — picks random image, avoids current. Uses `rand::seq::IndexedRandom` (rand 0.9 API)
- `delete_current()` — removes current from cache, updates cache file

### protocol.rs

cmux V2 compatible newline-delimited JSON protocol.

```rust
Request { id: String, method: String, params: serde_json::Value }
Response { id: String, ok: bool, result: Option<Value>, error: Option<ResponseError> }
ResponseError { code: String, message: String }
```

Used by turm-cli for socket communication.

### state.rs

Application state model.

```rust
AppState {
    config: TurmConfig,
    sessions: Mutex<HashMap<String, PtySession>>,
    workspaces: Mutex<Vec<Workspace>>,
    active_workspace: Mutex<Option<String>>,
}

Workspace { id, name, sessions: Vec<String>, focused_session: Option<String> }
```

**Note:** On Linux, VTE handles PTY internally. This state model is used for socket server features and is not yet wired into turm-linux.

### pty.rs

Cross-platform PTY session using `portable-pty`.

```rust
PtySession { master, child, input_tx: mpsc::Sender<Vec<u8>> }
```

- Input: mpsc channel → dedicated writer thread (no Mutex on hot path)
- Output: reader thread → callback function
- Buffer: 64KB reads

**Note:** Not used by turm-linux (VTE handles PTY). Intended for macOS and future socket server.

### error.rs

```rust
enum TurmError { Pty, Io, Config, SessionNotFound, Protocol }
type Result<T> = std::result::Result<T, TurmError>;
```
