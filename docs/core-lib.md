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

### error.rs

```rust
enum TurmError { Io, Config, Protocol }
type Result<T> = std::result::Result<T, TurmError>;
```

### event_bus.rs

In-process pub/sub hub for the workflow runtime (see [workflow-runtime.md](./workflow-runtime.md)). All internal event sources (shell signals, VTE output, service providers) publish through this bus; the socket `event.subscribe` stream becomes a projection of it.

```rust
Event { kind: String, source: String, timestamp_ms: u64, payload: Value }
EventBus::new() / with_default_buffer(n)
EventBus::publish(event)
EventBus::subscribe(pattern) -> EventReceiver
EventBus::subscribe_with_buffer(pattern, n) -> EventReceiver
EventReceiver::try_recv() -> Option<Event>
EventReceiver::recv() -> Option<Event>
```

**Pattern matching:** `*` matches all, `foo.*` matches any kind starting with `foo.` (deep — `foo.bar.baz` matches), otherwise exact string match.

**Delivery:** bounded `sync_channel` per subscriber (default buffer 256). On full buffer, the new event is dropped for that subscriber (`try_send`) with a warn log — publisher never blocks. Disconnected subscribers are pruned lazily on the next publish.

**Thread safety:** `EventBus` is `Sync`; any thread can publish. Receivers are single-consumer (not `Clone`) — platform UIs drain via `try_recv` on their main thread (GTK: `glib::timeout_add_local`; AppKit: DispatchSource).
