# Troubleshooting

## Build Issues

### Missing vte4 system library

```
error: could not find system library 'vte-2.91-gtk4'
```

**Fix:** `sudo pacman -S vte4`

### Missing gtk4 system library

**Fix:** `sudo pacman -S gtk4`

### `load_from_string` not found on CssProvider

The method is gated behind a feature flag.
**Fix:** Add `features = ["gnome_46"]` to gtk4 dependency in Cargo.toml.

### Cargo binary name collision

```
warning: output filename collision at target/debug/turm
```

turm-linux and turm-cli both output `turm`.
**Fix:** CLI binary renamed to `turmctl` in turm-cli/Cargo.toml.

## Runtime Issues

### Wayland protocol error (Error 71)

```
Gdk-Message: Error 71 (Protocol error)
```

**Fix:** Set `GDK_BACKEND=x11` in environment or in main.rs.

### GBM buffer error

```
Failed to create GBM buffer of size 841x1352: Invalid argument
```

**Fix:** Set `WEBKIT_DISABLE_DMABUF_RENDERER=1` (only relevant if using WebKit components).

### Terminal shows in light mode

**Cause:** Transparent VTE background with no image loaded shows system theme underneath.
**Fix:**

1. Force dark theme: `settings.set_gtk_application_prefer_dark_theme(true)` in `app.rs`
2. Set opaque VTE bg by default, only go transparent when bg image is applied

### Background images not showing (solid color only)

Multiple possible causes:

1. **Config `directory` is commented out**: Check `~/.config/turm/config.toml`. The `directory` field must be uncommented. A `#` before the key comments it out.

2. **VTE paints opaque background**: Call `terminal.set_clear_background(false)` in `set_background()`. Without this, VTE covers the image layer.

3. **Image loading fails silently**: The original `GtkPicture::set_file()` loads asynchronously and fails silently. Fixed by using `gdk::Texture::from_file()` for synchronous loading with error reporting.

4. **Tint too opaque**: Tint at 0.9 makes images nearly invisible (90% opaque dark overlay). Lower to 0.85 or less.

5. **GTK single-instance**: If an old turm is running, new launches activate the old instance and exit immediately (exit code 0, no output). Kill all instances first: `killall turm`.

### App exits immediately with no error

**Cause:** GTK single-instance behavior. Another turm instance already owns the GTK app ID `com.marshall.turm`.
**Fix:** `killall turm` then relaunch.

### env_logger output not visible

**Cause:** GTK may capture/redirect stderr. `RUST_LOG=info` has no visible effect.
**Fix:** Use `eprintln!("[turm] ...")` instead of `log::info!()` for debug output.

### Terminal shows only one line (collapsed height)

**Cause:** `GtkOverlay` sizes based on its child widget (`bg_picture`). When no background image is set, `bg_picture` is hidden and has zero natural size, collapsing the entire overlay.
**Fix:** Call `overlay.set_measure_overlay(&terminal, true)` so the terminal overlay widget contributes to size measurement even when `bg_picture` is hidden. Also set `overlay.set_hexpand(true)` and `overlay.set_vexpand(true)`.

### WebKit web process crashes on many sites

```
GStreamer element autoaudiosink not found. Please install it
GLib-GObject-CRITICAL: invalid (NULL) pointer instance
WebProcess CRASHED
```

**Cause:** Missing GStreamer plugins. WebKitGTK requires GStreamer for media handling, and crashes when the plugins are absent — even on pages that don't play media.
**Fix:** `sudo pacman -S gst-plugins-good gst-plugins-bad`

### D-Bus: `register_object` API mismatch

**Cause:** gio 0.20 uses builder pattern, not positional args.
**Fix:** Use `connection.register_object(path, &interface_info).method_call(closure).build()`.

---

## macOS App Issues

### SwiftTerm: `processTerminated` never called after shell exits

**Cause:** SwiftTerm's `LocalProcess.childProcessRead` detects PTY EOF and calls `childStopped()`, which cancels the internal `childMonitor` DispatchSource before it can fire. The `processTerminated` call in the EOF handler is commented out in SwiftTerm source.

**Fix:** Install a separate `DispatchSource.makeProcessSource` after `startProcess()` returns (in `TurmTerminalView.installExitMonitor()`). This source is not affected by `childStopped()` and fires independently when the process exits.

```swift
func installExitMonitor() {
    let pid = process.shellPid
    guard pid > 0 else { return }
    let src = DispatchSource.makeProcessSource(identifier: pid, eventMask: .exit, queue: .main)
    src.setEventHandler { [weak self, weak src] in
        src?.cancel()
        guard let self else { return }
        processDelegate?.processTerminated(source: self, exitCode: nil)
    }
    exitMonitor = src
    src.activate()
}
```

### macOS split panes: new pane gets wrong initial size

**Cause 1 (`layout()` approach):** NSSplitView calls `resizeSubviews` (which sets subview frames) before calling `layout()`. By the time `layout()` fires, the wrong frames are already committed. Calling `setPosition` in `layout()` fires too late — if the terminal view already has a large frame from before the rebuild, NSSplitView uses that as the basis for proportional sizing.

**Cause 2 (`asyncAfter` approach):** The 50ms delay is unreliable — layout may not have resolved yet, or a subsequent split may have started before the timer fires, applying stale positions.

**Fix:** Use `NSSplitViewDelegate.splitView(_:resizeSubviewsWithOldSize:)`. This delegate method is called by NSSplitView at the exact moment it needs to determine subview frames. Set frames directly here and set `initialSizeSet = true` after the first call to fall back to `adjustSubviews()` for subsequent resizes (preserving user drag behaviour).

### macOS: `becomeFirstResponder` cannot be overridden in SwiftTerm subclass

**Cause:** `MacTerminalView.becomeFirstResponder` is declared `public` but not `open`, so it cannot be overridden by code outside the SwiftTerm module.

**Fix:** Use `NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown)` in `PaneManager` to detect which pane was clicked and update `activePane` accordingly.

### macOS: `terminal.output` event not implementable

**Cause:** SwiftTerm's `feed(byteArray:)` is declared in an extension of `TerminalView` (not `open`), so it cannot be overridden by subclasses outside the module. There is no other public hook for intercepting raw PTY output bytes.

**Status:** Not implemented. Shell integration signals (`terminal.shell_precmd` / `terminal.shell_preexec`) are sent via socket commands from the shell script directly instead of OSC 133 parsing.

### macOS: Nerd Font icons show as boxes or render broken

**Cause 1 — Font not found by family name:** `NSFont(name:size:)` only accepts PostScript names and full names (e.g. `JetBrains Mono Regular`), not bare family names like `JetBrainsMono Nerd Font Mono`. When the lookup fails, the terminal falls back to the system monospace font which has no Nerd Font PUA glyphs.

**Fix:** Font resolution now uses a multi-step strategy: PostScript name → `NSFontManager` exact family lookup → case-insensitive family lookup → `NSFontDescriptor` → system fallback. Both PostScript names and family names now work reliably.

**Cause 2 — Using non-Mono Nerd Font variant:** Standard Nerd Font variants (e.g. `JetBrainsMono Nerd Font`) render icons as 2-column wide glyphs. SwiftTerm's Unicode width table does not include PUA codepoints (U+E000–U+F8FF), so it treats them as 1-column, causing icons to overflow into the adjacent cell.

**Fix:** Use the **Mono** variant of your Nerd Font (e.g. `JetBrainsMono Nerd Font Mono`). Mono variants explicitly set all icons to 1-column width.

```toml
[terminal]
font_family = "JetBrainsMono Nerd Font Mono"
```

### macOS: OSC 7 CWD URI includes hostname

**Cause:** OSC 7 delivers a `file://hostname/path` URI (e.g. `file://Marshalls-MacBook-Pro.local/Users/marshallku`). Simply stripping `file://` leaves the hostname in the path.

**Fix:** Use `URL(string: directory).path` to correctly extract only the POSIX path component, discarding the scheme and hostname.
