use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::gdk;
use gtk4::glib;
use vte4::prelude::*;
use vte4::Terminal;

use custerm_core::config::CustermConfig;

use crate::panel::Panel;

const PALETTE: &[&str] = &[
    "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af",
    "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
    "#585b70", "#f38ba8", "#a6e3a1", "#f9e2af",
    "#89b4fa", "#f5c2e7", "#94e2d5", "#a6adc8",
];

const DEFAULT_FONT_SCALE: f64 = 1.0;
const FONT_SCALE_STEP: f64 = 0.1;
const MIN_FONT_SCALE: f64 = 0.3;
const MAX_FONT_SCALE: f64 = 3.0;

pub struct TerminalPanel {
    pub overlay: gtk4::Overlay,
    pub terminal: Terminal,
    pub bg_picture: gtk4::Picture,
    pub tint_overlay: gtk4::Box,
    pub tint_css: gtk4::CssProvider,
    pub tint_opacity: Rc<Cell<f64>>,
    pub tint_color: Rc<Cell<gdk::RGBA>>,
    pub image_opacity: Rc<Cell<f64>>,
    pub has_background: Rc<Cell<bool>>,
}

impl TerminalPanel {
    pub fn new(config: &CustermConfig, on_exit: impl Fn() + 'static) -> Self {
        let terminal = Terminal::new();

        // Font
        let font_desc = gtk4::pango::FontDescription::from_string(
            &format!("{} {}", config.terminal.font_family, config.terminal.font_size),
        );
        terminal.set_font(Some(&font_desc));
        terminal.set_font_scale(DEFAULT_FONT_SCALE);

        // Colors
        let fg = parse_color("#cdd6f4");
        let bg = parse_color("#1e1e2e");
        let palette = make_palette();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);

        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::On);
        terminal.set_cursor_shape(vte4::CursorShape::Block);
        terminal.set_scrollback_lines(10000);
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Zoom shortcuts
        let zoom_controller = gtk4::EventControllerKey::new();
        let term_clone = terminal.clone();
        zoom_controller.connect_key_pressed(move |_, keyval, _, modifier| {
            if !modifier.contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            match keyval {
                gdk::Key::equal | gdk::Key::plus => {
                    let scale = (term_clone.font_scale() + FONT_SCALE_STEP).min(MAX_FONT_SCALE);
                    term_clone.set_font_scale(scale);
                    glib::Propagation::Stop
                }
                gdk::Key::minus => {
                    let scale = (term_clone.font_scale() - FONT_SCALE_STEP).max(MIN_FONT_SCALE);
                    term_clone.set_font_scale(scale);
                    glib::Propagation::Stop
                }
                gdk::Key::_0 => {
                    term_clone.set_font_scale(DEFAULT_FONT_SCALE);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        terminal.add_controller(zoom_controller);

        // Spawn shell
        let shell = config.terminal.shell.clone();
        let dbus_env = format!("CUSTERM_DBUS={}", crate::dbus::bus_name());
        terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None::<&str>,
            &[&shell],
            &[&dbus_env],
            gtk4::glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gtk4::gio::Cancellable::NONE,
            |_result| {},
        );

        terminal.connect_child_exited(move |_terminal, _status| {
            on_exit();
        });

        // Background image layer (GPU-rendered)
        let image_opacity = Rc::new(Cell::new(config.background.opacity));
        let bg_picture = gtk4::Picture::new();
        bg_picture.set_content_fit(gtk4::ContentFit::Cover);
        bg_picture.set_hexpand(true);
        bg_picture.set_vexpand(true);
        bg_picture.set_visible(false);
        bg_picture.set_opacity(config.background.opacity);

        // Tint overlay (CSS-driven)
        let tint_opacity = Rc::new(Cell::new(config.background.tint));
        let tint_color = Rc::new(Cell::new(parse_color(&config.background.tint_color)));
        let tint_overlay = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        tint_overlay.set_hexpand(true);
        tint_overlay.set_vexpand(true);
        tint_overlay.set_visible(false);
        let tint_css = gtk4::CssProvider::new();
        update_tint_css(&tint_css, &config.background.tint_color, config.background.tint);
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &tint_css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );
        tint_overlay.add_css_class("custerm-tint");

        // VTE transparent CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("vte-terminal { background-color: transparent; }");
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );

        // Stack: bg_picture → tint → terminal
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&bg_picture));
        overlay.add_overlay(&tint_overlay);
        overlay.add_overlay(&terminal);

        Self {
            overlay,
            terminal,
            bg_picture,
            tint_overlay,
            tint_css,
            tint_opacity,
            tint_color,
            image_opacity,
            has_background: Rc::new(Cell::new(false)),
        }
    }

    pub fn set_background(&self, path: &Path) {
        eprintln!("[custerm] set_background: {}", path.display());

        if !path.exists() {
            eprintln!("[custerm] file does not exist: {}", path.display());
            return;
        }

        let file = gtk4::gio::File::for_path(path);
        match gdk::Texture::from_file(&file) {
            Ok(texture) => {
                eprintln!(
                    "[custerm] loaded texture: {}x{}",
                    texture.width(),
                    texture.height()
                );
                self.bg_picture.set_paintable(Some(&texture));
            }
            Err(e) => {
                eprintln!("[custerm] FAILED to load image {}: {}", path.display(), e);
                return;
            }
        }

        self.bg_picture.set_visible(true);
        self.bg_picture.set_opacity(self.image_opacity.get());
        self.tint_overlay.set_visible(true);
        self.has_background.set(true);

        self.terminal.set_clear_background(false);
        let fg = parse_color("#cdd6f4");
        let bg = gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
        let palette = make_palette();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        self.terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);
        self.terminal.set_color_background(&bg);
    }

    pub fn clear_background(&self) {
        eprintln!("[custerm] clear_background");
        self.bg_picture.set_visible(false);
        self.tint_overlay.set_visible(false);
        self.has_background.set(false);

        self.terminal.set_clear_background(true);

        let fg = parse_color("#cdd6f4");
        let bg = parse_color("#1e1e2e");
        let palette = make_palette();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        self.terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);
    }

    pub fn set_tint(&self, opacity: f64) {
        self.tint_opacity.set(opacity);
        let c = self.tint_color.get();
        update_tint_css(&self.tint_css, &format!("#{:02x}{:02x}{:02x}",
            (c.red() * 255.0) as u8,
            (c.green() * 255.0) as u8,
            (c.blue() * 255.0) as u8,
        ), opacity);
    }

    pub fn apply_config(&self, config: &CustermConfig) {
        let font_desc = gtk4::pango::FontDescription::from_string(
            &format!("{} {}", config.terminal.font_family, config.terminal.font_size),
        );
        self.terminal.set_font(Some(&font_desc));

        self.tint_opacity.set(config.background.tint);
        self.tint_color.set(parse_color(&config.background.tint_color));
        update_tint_css(&self.tint_css, &config.background.tint_color, config.background.tint);

        self.image_opacity.set(config.background.opacity);
        if self.has_background.get() {
            self.bg_picture.set_opacity(config.background.opacity);
        }

        match &config.background.image {
            Some(image) => {
                let path = Path::new(image);
                if path.exists() {
                    self.set_background(path);
                }
            }
            None => {
                if self.has_background.get() {
                    self.clear_background();
                }
            }
        }
    }
}

impl Panel for TerminalPanel {
    fn widget(&self) -> &gtk4::Widget {
        self.overlay.upcast_ref()
    }

    fn title(&self) -> String {
        self.terminal.window_title()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "Terminal".to_string())
    }

    fn panel_type(&self) -> &str {
        "terminal"
    }

    fn grab_focus(&self) {
        self.terminal.grab_focus();
    }
}

fn update_tint_css(provider: &gtk4::CssProvider, hex_color: &str, opacity: f64) {
    let c = parse_color(hex_color);
    let css = format!(
        ".custerm-tint {{ background-color: rgba({},{},{},{}); }}",
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8,
        opacity,
    );
    provider.load_from_string(&css);
}

fn make_palette() -> Vec<gdk::RGBA> {
    PALETTE.iter().map(|c| parse_color(c)).collect()
}

fn parse_color(hex: &str) -> gdk::RGBA {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    gdk::RGBA::new(r, g, b, 1.0)
}
