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
    pub bg_drawing: gtk4::DrawingArea,
    pub tint_overlay: gtk4::DrawingArea,
    pub tint_opacity: Rc<Cell<f64>>,
    pub tint_color: Rc<Cell<gdk::RGBA>>,
    pub bg_texture: Rc<Cell<Option<gdk::Texture>>>,
    pub image_opacity: Rc<Cell<f64>>,
    pub has_background: Rc<Cell<bool>>,
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

        // Colors - Catppuccin Mocha, opaque by default
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

        // Spawn shell with CUSTERM_DBUS env var for per-session control
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

        terminal.connect_child_exited(|terminal, _status| {
            if let Some(toplevel) = terminal.root() {
                if let Some(window) = toplevel.downcast_ref::<gtk4::Window>() {
                    window.close();
                }
            }
        });

        // Background image layer
        let image_opacity = Rc::new(Cell::new(config.background.opacity));
        let bg_texture: Rc<Cell<Option<gdk::Texture>>> = Rc::new(Cell::new(None));
        let bg_drawing = gtk4::DrawingArea::new();
        bg_drawing.set_hexpand(true);
        bg_drawing.set_vexpand(true);
        bg_drawing.set_visible(false);

        let tex_ref = bg_texture.clone();
        let opacity_ref = image_opacity.clone();
        bg_drawing.set_draw_func(move |_widget, cr, width, height| {
            let texture = tex_ref.take();
            if let Some(ref tex) = texture {
                let tw = tex.width() as f64;
                let th = tex.height() as f64;
                let w = width as f64;
                let h = height as f64;

                let scale = (w / tw).max(h / th);
                let ox = (w - tw * scale) / 2.0;
                let oy = (h - th * scale) / 2.0;

                let stride = (tex.width() as usize) * 4;
                let mut data = vec![0u8; stride * tex.height() as usize];
                tex.download(&mut data, stride);

                let surface = gtk4::cairo::ImageSurface::create_for_data(
                    data,
                    gtk4::cairo::Format::ARgb32,
                    tex.width(),
                    tex.height(),
                    stride as i32,
                ).unwrap();

                cr.save().unwrap();
                cr.translate(ox, oy);
                cr.scale(scale, scale);
                cr.set_source_surface(&surface, 0.0, 0.0).unwrap();
                // Paint image with configurable opacity (text stays fully opaque)
                cr.paint_with_alpha(opacity_ref.get()).unwrap();
                cr.restore().unwrap();
            }
            tex_ref.set(texture);
        });

        // Tint overlay
        let tint_opacity = Rc::new(Cell::new(config.background.tint));
        let tint_color = Rc::new(Cell::new(parse_color(&config.background.tint_color)));
        let tint_overlay = gtk4::DrawingArea::new();
        tint_overlay.set_hexpand(true);
        tint_overlay.set_vexpand(true);
        tint_overlay.set_visible(false);
        let tint_val = tint_opacity.clone();
        let tint_col = tint_color.clone();
        tint_overlay.set_draw_func(move |_, cr, width, height| {
            let c = tint_col.get();
            cr.set_source_rgba(
                c.red() as f64,
                c.green() as f64,
                c.blue() as f64,
                tint_val.get(),
            );
            cr.rectangle(0.0, 0.0, width as f64, height as f64);
            let _ = cr.fill();
        });

        // CSS to force VTE background transparent (text stays opaque)
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("vte-terminal { background-color: transparent; }");
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );

        // Stack: bg_drawing (base) -> tint_overlay -> terminal
        let overlay = gtk4::Overlay::new();
        overlay.set_child(Some(&bg_drawing));
        overlay.add_overlay(&tint_overlay);
        overlay.add_overlay(&terminal);

        Self {
            overlay,
            terminal,
            bg_drawing,
            tint_overlay,
            tint_opacity,
            tint_color,
            bg_texture,
            image_opacity,
            has_background: Rc::new(Cell::new(false)),
        }
    }

    pub fn widget(&self) -> &gtk4::Overlay {
        &self.overlay
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
                self.bg_texture.set(Some(texture));
            }
            Err(e) => {
                eprintln!("[custerm] FAILED to load image {}: {}", path.display(), e);
                return;
            }
        }

        self.bg_drawing.set_visible(true);
        self.bg_drawing.queue_draw();
        self.tint_overlay.set_visible(true);
        self.has_background.set(true);

        // Make VTE background transparent so image shows through, text stays opaque
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
        self.bg_drawing.set_visible(false);
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
        self.tint_overlay.queue_draw();
    }
}

fn make_palette() -> Vec<gdk::RGBA> {
    PALETTE.iter().map(|c| parse_color(c)).collect()
}

pub fn parse_color_pub(hex: &str) -> gdk::RGBA {
    parse_color(hex)
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
