use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::gdk;
use gtk4::glib;
use vte4::prelude::*;
use vte4::Terminal;

use custerm_core::config::CustermConfig;

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

pub struct TerminalTab {
    pub overlay: gtk4::Overlay,
    pub terminal: Terminal,
    pub bg_picture: gtk4::Picture,
    pub tint_overlay: gtk4::DrawingArea,
    pub tint_opacity: Rc<Cell<f64>>,
}

impl TerminalTab {
    pub fn new(config: &CustermConfig) -> Self {
        let terminal = Terminal::new();

        // Font
        let font_desc = gtk4::pango::FontDescription::from_string(
            &format!("{} {}", config.terminal.font_family, config.terminal.font_size),
        );
        terminal.set_font(Some(&font_desc));
        terminal.set_font_scale(DEFAULT_FONT_SCALE);

        // Colors - Catppuccin Mocha, opaque by default (made transparent when bg image is set)
        let fg = parse_color("#cdd6f4");
        let bg = parse_color("#1e1e2e");
        let palette: Vec<gdk::RGBA> = PALETTE.iter().map(|c| parse_color(c)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);

        terminal.set_cursor_blink_mode(vte4::CursorBlinkMode::On);
        terminal.set_cursor_shape(vte4::CursorShape::Block);
        terminal.set_scrollback_lines(10000);
        terminal.set_hexpand(true);
        terminal.set_vexpand(true);

        // Keyboard shortcuts: Ctrl+= zoom in, Ctrl+- zoom out, Ctrl+0 reset
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
        terminal.spawn_async(
            vte4::PtyFlags::DEFAULT,
            None::<&str>,
            &[&shell],
            &[] as &[&str],
            gtk4::glib::SpawnFlags::DEFAULT,
            || {},
            -1,
            gtk4::gio::Cancellable::NONE,
            |_result| {},
        );

        terminal.connect_child_exited(|terminal, _status| {
            if let Some(toplevel) = terminal.root() {
                if let Some(window) = toplevel.downcast_ref::<gtk4::Window>() {
                    window.close();
                }
            }
        });

        // Background image layer (hidden by default)
        let bg_picture = gtk4::Picture::new();
        bg_picture.set_content_fit(gtk4::ContentFit::Cover);
        bg_picture.set_hexpand(true);
        bg_picture.set_vexpand(true);
        bg_picture.set_visible(false);

        // Tint overlay (drawn on top of image, behind terminal)
        let tint_opacity = Rc::new(Cell::new(config.background.tint));
        let tint_overlay = gtk4::DrawingArea::new();
        tint_overlay.set_hexpand(true);
        tint_overlay.set_vexpand(true);
        tint_overlay.set_visible(false);
        let tint_val = tint_opacity.clone();
        tint_overlay.set_draw_func(move |_, cr, width, height| {
            cr.set_source_rgba(0.118, 0.118, 0.180, tint_val.get()); // #1e1e2e
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
        });

        // Stack: bg_picture -> tint_overlay -> terminal (via GtkOverlay)
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&bg_picture));
        overlay.add_overlay(&tint_overlay);
        overlay.add_overlay(&terminal);

        Self {
            overlay,
            terminal,
            bg_picture,
            tint_overlay,
            tint_opacity,
        }
    }

    pub fn widget(&self) -> &gtk4::Overlay {
        &self.overlay
    }

    pub fn set_background(&self, path: &Path) {
        let file = gtk4::gio::File::for_path(path);
        self.bg_picture.set_file(Some(&file));
        self.bg_picture.set_visible(true);
        self.tint_overlay.set_visible(true);

        // Stop VTE from painting its own opaque background
        self.terminal.set_clear_background(false);

        // Set transparent background color
        let fg = parse_color("#cdd6f4");
        let bg = gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
        let palette: Vec<gdk::RGBA> = PALETTE.iter().map(|c| parse_color(c)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        self.terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);
    }

    pub fn clear_background(&self) {
        self.bg_picture.set_visible(false);
        self.tint_overlay.set_visible(false);

        self.terminal.set_clear_background(true);

        let fg = parse_color("#cdd6f4");
        let bg = parse_color("#1e1e2e");
        let palette: Vec<gdk::RGBA> = PALETTE.iter().map(|c| parse_color(c)).collect();
        let palette_refs: Vec<&gdk::RGBA> = palette.iter().collect();
        self.terminal.set_colors(Some(&fg), Some(&bg), &palette_refs);
    }

    pub fn set_tint(&self, opacity: f64) {
        self.tint_opacity.set(opacity);
        self.tint_overlay.queue_draw();
    }
}

fn parse_color(hex: &str) -> gdk::RGBA {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    gdk::RGBA::new(r, g, b, 1.0)
}
