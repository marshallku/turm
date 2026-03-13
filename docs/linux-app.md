# Linux App (turm-linux)

## Entry Point (`main.rs`)

CLI flags handled before GTK launch:

- `--init-config` — writes default config to `~/.config/turm/config.toml`
- `--config-path` — prints config file path

## Application (`app.rs`)

- GTK Application ID: `com.marshall.turm`
- Forces dark theme on startup via `set_gtk_application_prefer_dark_theme(true)`
- Loads config with `TurmConfig::load()`, falls back to defaults

## Window (`window.rs`)

- Default size: 1200x800
- CSS: `window { background-color: #1e1e2e; }` (Catppuccin Mocha base)
- Creates a single `TerminalTab` and sets it as window child
- Initializes `BackgroundManager` and applies first random background if directory is configured
- Registers D-Bus service and polls for commands every 50ms via `glib::timeout_add_local`

### D-Bus Command Loop

```
D-Bus callback (any thread) → mpsc::channel → glib::timeout_add_local (GTK main thread) → widget updates
```

This pattern is required because GTK widgets are not `Send+Sync` and can only be accessed from the main thread.

## Terminal (`terminal.rs`)

### TerminalPanel Struct

```rust
pub struct TerminalPanel {
    pub overlay: gtk4::Overlay,
    pub terminal: vte4::Terminal,
    pub bg_picture: gtk4::Picture,
    pub tint_overlay: gtk4::Box,
    pub tint_css: gtk4::CssProvider,
    pub tint_opacity: Rc<Cell<f64>>,
    pub tint_color: Rc<Cell<gdk::RGBA>>,
    pub image_opacity: Rc<Cell<f64>>,
    pub has_background: Rc<Cell<bool>>,
    pub search_bar: SearchBar,
}
```

### Overlay Stack (bottom to top)

```
bg_picture (GtkPicture, content-fit: cover)  ← child of overlay
  └─ tint_overlay (Box, CSS rgba)            ← overlay
      └─ terminal (VTE Terminal)             ← overlay (set_measure_overlay=true)
          └─ search_bar (Box, valign=End)    ← overlay (hidden by default)
```

**Critical:** `overlay.set_measure_overlay(&terminal, true)` ensures the terminal contributes to overlay size measurement. Without this, when `bg_picture` is hidden (no background image), the overlay collapses to zero height since the child has no natural size.

### Font Scaling

- Keyboard: `Ctrl+=` zoom in, `Ctrl+-` zoom out, `Ctrl+0` reset
- Range: 0.3x to 3.0x, step 0.1
- Uses `terminal.set_font_scale()`

### Background Image Compositing

**`set_background(path)`:**

1. Sets `bg_picture` file and makes it visible
2. Shows `tint_overlay`
3. Calls `terminal.set_clear_background(false)` — **critical**: stops VTE from painting opaque bg
4. Sets VTE background color to fully transparent `RGBA(0, 0, 0, 0)`

**`clear_background()`:**

1. Hides `bg_picture` and `tint_overlay`
2. Calls `terminal.set_clear_background(true)` — re-enables VTE opaque bg
3. Restores opaque Catppuccin Mocha background color

**`set_tint(opacity)`:**

- Updates `tint_opacity` Rc<Cell> and queues redraw

### Color Palette

Catppuccin Mocha 16-color palette:

- Foreground: `#cdd6f4`
- Background: `#1e1e2e` (opaque) / `rgba(0,0,0,0)` (with bg image)
- See `PALETTE` constant and `parse_color()` function

### Shell Spawn

VTE handles PTY internally via `terminal.spawn_async()`. No custom PTY management needed on Linux.

On child exit, the window closes automatically via `connect_child_exited`.

## Tabs (`tabs.rs`)

### TabManager

Manages `gtk4::Notebook` with `TabContent` entries (split pane trees).

- Tab position configurable via `[tabs] position` in config (`top`, `bottom`, `left`, `right`)
- Tab bar has collapsed (icon-only) and expanded modes, toggled via Ctrl+Shift+B
- Tab position and collapsed state hot-reload on config change

### Keyboard Shortcuts

All built-in shortcuts use `Ctrl+Shift` — Ctrl-only keys pass through to terminal/webview.

| Shortcut                            | Action                              |
| ----------------------------------- | ----------------------------------- |
| `Ctrl+Shift+B`                      | Toggle tab bar (collapsed/expanded) |
| `Ctrl+Shift+F`                      | Toggle search bar                   |
| `Ctrl+Shift+T`                      | New tab                             |
| `Ctrl+Shift+W`                      | Close focused pane/tab              |
| `Ctrl+Shift+C`                      | Copy (terminal)                     |
| `Ctrl+Shift+V`                      | Paste (terminal)                    |
| `Ctrl+Shift+E`                      | Split horizontal                    |
| `Ctrl+Shift+O`                      | Split vertical                      |
| `Ctrl+Shift+N` / `Ctrl+Shift+Right` | Focus next pane                     |
| `Ctrl+Shift+P` / `Ctrl+Shift+Left`  | Focus previous pane                 |
| `Ctrl+Shift+Tab`                    | Next tab                            |
| `Ctrl+Shift+1-9`                    | Switch to tab N                     |

### Custom Keybindings

Custom keybindings can be configured in `[keybindings]` section of `config.toml`. They are checked before built-in shortcuts, so they can override defaults. Spawned commands receive `TURM_DBUS` and `TURM_SOCKET` environment variables. See [config.md](./config.md#keybindings) for details.

## Search (`search.rs`)

In-terminal text search using VTE's built-in regex search API.

- **Toggle:** `Ctrl+Shift+F` opens/closes the search bar (overlay at bottom of terminal)
- **Search:** Uses `vte4::Regex::for_search()` with PCRE2, applied via `terminal.search_set_regex()`
- **Navigation:** `Enter` = next match, `Shift+Enter` = previous match
- **Close:** `Escape` closes search and returns focus to terminal
- **Case sensitivity:** Toggle button, default is case-insensitive (`PCRE2_CASELESS`)
- **Reopen behavior:** Previous search text is preserved but fully selected, so typing immediately replaces it
- **Wrap around:** Enabled by default

## Split Panes (`split.rs`)

Binary tree of `SplitNode` (Leaf = terminal, Branch = `gtk4::Paned` with two children). Each tab has a `TabContent` with a root `SplitNode` and a stable container `gtk4::Box`.

## D-Bus Interface (`dbus.rs`)

Bus name: `com.marshall.turm`
Object path: `/com/marshall/turm`

### Methods

| Method                 | Args           | Description                            |
| ---------------------- | -------------- | -------------------------------------- |
| `SetBackground`        | `path: String` | Set specific background image          |
| `NextBackground`       | —              | Random next from cache                 |
| `ClearBackground`      | —              | Remove background, restore solid color |
| `SetTint`              | `opacity: f64` | Set tint overlay opacity               |
| `GetCurrentBackground` | —              | Returns current image path             |

### Testing D-Bus

```bash
# Next random background
gdbus call --session -d com.marshall.turm -o /com/marshall/turm -m com.marshall.turm.NextBackground

# Get current background
gdbus call --session -d com.marshall.turm -o /com/marshall/turm -m com.marshall.turm.GetCurrentBackground

# Set tint
gdbus call --session -d com.marshall.turm -o /com/marshall/turm -m com.marshall.turm.SetTint 0.7

# Set specific image
gdbus call --session -d com.marshall.turm -o /com/marshall/turm -m com.marshall.turm.SetBackground "/path/to/image.jpg"

# Clear background
gdbus call --session -d com.marshall.turm -o /com/marshall/turm -m com.marshall.turm.ClearBackground
```

## Installation

```bash
# Build + install
./turm-linux/install.sh

# Or manually
cargo build --release -p turm-linux
sudo install -Dm755 target/release/turm /usr/local/bin/turm
sudo install -Dm644 turm-linux/turm.desktop /usr/share/applications/turm.desktop

# Set as default terminal (GNOME)
gsettings set org.gnome.desktop.default-applications.terminal exec turm
```
