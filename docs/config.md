# Configuration

Path: `~/.config/turm/config.toml`

## Generate Default Config

```bash
turm --init-config
```

## Print Config Path

```bash
turm --config-path
```

## Full Example

```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrainsMono Nerd Font Mono"
font_size = 14

[background]
# image = "/path/to/wallpaper.jpg"  # Single image (takes priority)
directory = "/mnt/disk2/Wallpapers/"  # Directory for random picks
tint = 0.85         # Tint overlay opacity (0.0 = no tint, 1.0 = fully opaque)
opacity = 0.95      # Terminal opacity

[tabs]
position = "left"   # top, bottom, left, right

[socket]
path = "/tmp/turm.sock"

[theme]
name = "catppuccin-mocha"
```

## Sections

### [terminal]

| Key           | Default                        | Description         |
| ------------- | ------------------------------ | ------------------- |
| `shell`       | `$SHELL` or `/bin/sh`          | Shell to spawn      |
| `font_family` | `JetBrainsMono Nerd Font Mono` | Font family         |
| `font_size`   | `14`                           | Font size in points |

### [background]

| Key         | Default      | Description                                              |
| ----------- | ------------ | -------------------------------------------------------- |
| `image`     | — (optional) | Single image file path (takes priority over directory)   |
| `directory` | — (optional) | Path to wallpaper directory (random pick)                |
| `tint`      | `0.9`        | Tint overlay opacity (0.0=transparent, 1.0=fully opaque) |
| `opacity`   | `0.95`       | Terminal opacity                                         |

### [tabs]

| Key        | Default | Description                                        |
| ---------- | ------- | -------------------------------------------------- |
| `position` | `top`   | Tab bar position: `top`, `bottom`, `left`, `right` |

### [socket]

| Key    | Default          | Description      |
| ------ | ---------------- | ---------------- |
| `path` | `/tmp/turm.sock` | Unix socket path |

### [theme]

| Key    | Default            | Description |
| ------ | ------------------ | ----------- |
| `name` | `catppuccin-mocha` | Theme name  |

## Notes

- All fields have defaults; config file is optional
- Missing sections are filled with defaults via `#[serde(default)]`
- Config hot-reloads automatically via file watcher (font, background, tint, tab position)
