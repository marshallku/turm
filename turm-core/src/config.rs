use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;

fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

fn default_font_family() -> String {
    "JetBrainsMono Nerd Font Mono".to_string()
}

fn default_font_size() -> u32 {
    14
}

fn default_tint() -> f64 {
    0.85
}

fn default_tint_color() -> String {
    "#1e1e2e".to_string()
}

fn default_opacity() -> f64 {
    0.95
}

fn default_tab_position() -> String {
    "top".to_string()
}

fn default_tab_width() -> u32 {
    120
}

fn default_theme() -> String {
    "catppuccin-mocha".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    #[serde(default = "default_shell")]
    pub shell: String,

    #[serde(default = "default_font_family")]
    pub font_family: String,

    #[serde(default = "default_font_size")]
    pub font_size: u32,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            shell: default_shell(),
            font_family: default_font_family(),
            font_size: default_font_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundConfig {
    #[serde(default)]
    pub image: Option<String>,

    #[serde(default = "default_tint")]
    pub tint: f64,

    #[serde(default = "default_tint_color")]
    pub tint_color: String,

    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            image: None,
            tint: default_tint(),
            tint_color: default_tint_color(),
            opacity: default_opacity(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_theme")]
    pub name: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: default_theme(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabsConfig {
    /// Tab bar position: "top", "bottom", "left", "right"
    #[serde(default = "default_tab_position")]
    pub position: String,
    /// Width of vertical tabs in pixels (left/right position)
    #[serde(default = "default_tab_width")]
    pub width: u32,
    /// Whether the tab bar starts collapsed (icon-only). Default: true
    #[serde(default = "default_true")]
    pub collapsed: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TabsConfig {
    fn default() -> Self {
        Self {
            position: default_tab_position(),
            width: default_tab_width(),
            collapsed: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TurmConfig {
    #[serde(default)]
    pub terminal: TerminalConfig,

    #[serde(default)]
    pub background: BackgroundConfig,

    #[serde(default)]
    pub tabs: TabsConfig,

    #[serde(default)]
    pub theme: ThemeConfig,
}

impl TurmConfig {
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("/etc"))
            .join("turm")
            .join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(&config_path)?;
        let config: TurmConfig = toml::from_str(&contents)
            .map_err(|e| crate::error::TurmError::Config(e.to_string()))?;

        Ok(config)
    }

    pub fn write_default() -> Result<PathBuf> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let default_config = r##"[terminal]
# shell = "/bin/zsh"
font_family = "JetBrainsMono Nerd Font Mono"
font_size = 14

[background]
# image = "/path/to/wallpaper.jpg"
# tint = 0.85
# tint_color = "#1e1e2e"
# opacity = 0.95

[tabs]
# position = "top"  # top, bottom, left, right
# width = 120       # vertical tab width in pixels (left/right)
# collapsed = true  # start with tab bar collapsed (icon-only)

[theme]
# Available: catppuccin-mocha, catppuccin-latte, catppuccin-frappe, catppuccin-macchiato,
#            dracula, nord, tokyo-night, gruvbox-dark, one-dark, solarized-dark
name = "catppuccin-mocha"
"##;
        std::fs::write(&path, default_config)?;
        Ok(path)
    }
}
