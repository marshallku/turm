use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::prelude::*;

use turm_core::config::TurmConfig;

use crate::terminal::parse_color;

/// Window-level background layer: image + tint that sit underneath every
/// tab/panel. Replaces the per-`TerminalPanel` background that only ever
/// rendered inside one terminal's overlay.
///
/// Mounted as the base child of a `gtk4::Overlay` in `TurmWindow`; the rest
/// of the UI (statusbar, notebook, panels) is added on top as overlays. Every
/// non-terminal surface (notebook header, statusbar, plugin webviews) is
/// painted transparent so this layer shows through consistently.
pub struct BackgroundLayer {
    pub bg_picture: gtk4::Picture,
    pub tint_overlay: gtk4::Box,
    tint_css: gtk4::CssProvider,
    tint_opacity: Cell<f64>,
    tint_color: Cell<gdk::RGBA>,
    image_opacity: Cell<f64>,
    has_image: Cell<bool>,
}

impl BackgroundLayer {
    pub fn new(config: &TurmConfig) -> Rc<Self> {
        let bg_picture = gtk4::Picture::new();
        bg_picture.set_content_fit(gtk4::ContentFit::Cover);
        bg_picture.set_hexpand(true);
        bg_picture.set_vexpand(true);
        bg_picture.set_visible(false);
        bg_picture.set_opacity(config.background.opacity);
        // Don't intercept input — clicks must reach the panels above.
        bg_picture.set_can_target(false);

        let tint_overlay = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        tint_overlay.set_hexpand(true);
        tint_overlay.set_vexpand(true);
        tint_overlay.set_visible(false);
        tint_overlay.set_can_target(false);
        tint_overlay.add_css_class("turm-bg-tint");

        let tint_css = gtk4::CssProvider::new();
        update_tint_css(
            &tint_css,
            &config.background.tint_color,
            config.background.tint,
        );
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().unwrap(),
            &tint_css,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
        );

        let layer = Rc::new(Self {
            bg_picture,
            tint_overlay,
            tint_css,
            tint_opacity: Cell::new(config.background.tint),
            tint_color: Cell::new(parse_color(&config.background.tint_color)),
            image_opacity: Cell::new(config.background.opacity),
            has_image: Cell::new(false),
        });

        if let Some(ref path) = config.background.image {
            let p = Path::new(path);
            if p.exists() {
                layer.set_image(p);
            }
        }

        layer
    }

    pub fn set_image(&self, path: &Path) {
        eprintln!("[turm] background.set_image: {}", path.display());

        if !path.exists() {
            eprintln!("[turm] background image does not exist: {}", path.display());
            return;
        }

        let file = gtk4::gio::File::for_path(path);
        match gdk::Texture::from_file(&file) {
            Ok(texture) => {
                eprintln!(
                    "[turm] background texture loaded: {}x{}",
                    texture.width(),
                    texture.height()
                );
                self.bg_picture.set_paintable(Some(&texture));
            }
            Err(e) => {
                eprintln!(
                    "[turm] FAILED to load background image {}: {}",
                    path.display(),
                    e
                );
                return;
            }
        }

        self.bg_picture.set_visible(true);
        self.bg_picture.set_opacity(self.image_opacity.get());
        self.tint_overlay.set_visible(true);
        self.has_image.set(true);
    }

    pub fn clear_image(&self) {
        eprintln!("[turm] background.clear_image");
        self.bg_picture.set_visible(false);
        self.tint_overlay.set_visible(false);
        self.has_image.set(false);
    }

    pub fn set_tint(&self, opacity: f64) {
        self.tint_opacity.set(opacity);
        let c = self.tint_color.get();
        update_tint_css(
            &self.tint_css,
            &format!(
                "#{:02x}{:02x}{:02x}",
                (c.red() * 255.0) as u8,
                (c.green() * 255.0) as u8,
                (c.blue() * 255.0) as u8,
            ),
            opacity,
        );
    }

    pub fn apply_config(&self, config: &TurmConfig) {
        self.tint_opacity.set(config.background.tint);
        self.tint_color
            .set(parse_color(&config.background.tint_color));
        update_tint_css(
            &self.tint_css,
            &config.background.tint_color,
            config.background.tint,
        );

        self.image_opacity.set(config.background.opacity);
        if self.has_image.get() {
            self.bg_picture.set_opacity(config.background.opacity);
        }

        match &config.background.image {
            Some(image) => {
                let path = Path::new(image);
                if path.exists() {
                    self.set_image(path);
                }
            }
            None => {
                if self.has_image.get() {
                    self.clear_image();
                }
            }
        }
    }
}

fn update_tint_css(provider: &gtk4::CssProvider, hex_color: &str, opacity: f64) {
    let c = parse_color(hex_color);
    let css = format!(
        ".turm-bg-tint {{ background-color: rgba({},{},{},{}); }}",
        (c.red() * 255.0) as u8,
        (c.green() * 255.0) as u8,
        (c.blue() * 255.0) as u8,
        opacity,
    );
    provider.load_from_string(&css);
}
