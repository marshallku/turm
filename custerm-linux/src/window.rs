use std::path::PathBuf;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{Application, ApplicationWindow, gio, glib};
use vte4::prelude::*;

use custerm_core::config::CustermConfig;

use crate::dbus::{self, DbusCommand};
use crate::terminal::TerminalTab;

pub struct CustermWindow {
    pub window: ApplicationWindow,
}

impl CustermWindow {
    pub fn new(app: &Application, config: &CustermConfig) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("custerm")
            .default_width(1200)
            .default_height(800)
            .build();

        let terminal = TerminalTab::new(config);
        window.set_child(Some(terminal.widget()));

        // Apply CSS
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_string("window { background-color: #1e1e2e; }");
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().unwrap(),
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Apply initial background from config
        if let Some(path) = config.background.image.as_ref().map(PathBuf::from) {
            if path.exists() {
                terminal.set_background(&path);
            }
        }

        // Watch config file for changes (hot-reload)
        watch_config(&terminal);

        // Register D-Bus and poll for commands on main thread
        let rx = dbus::register();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(cmd) = rx.try_recv() {
                match cmd {
                    DbusCommand::SetBackground(path) => {
                        terminal.set_background(std::path::Path::new(&path));
                    }
                    DbusCommand::ClearBackground => {
                        terminal.clear_background();
                    }
                    DbusCommand::SetTint(opacity) => {
                        terminal.set_tint(opacity);
                    }
                }
            }
            glib::ControlFlow::Continue
        });

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}

fn watch_config(terminal: &TerminalTab) {
    let config_path = CustermConfig::config_path();
    let file = gio::File::for_path(&config_path);

    let monitor = match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[custerm] failed to watch config: {}", e);
            return;
        }
    };

    let tint_opacity = terminal.tint_opacity.clone();
    let tint_color = terminal.tint_color.clone();
    let tint_overlay = terminal.tint_overlay.clone();
    let image_opacity = terminal.image_opacity.clone();
    let term = terminal.terminal.clone();
    let has_bg = terminal.has_background.clone();
    let bg_texture = terminal.bg_texture.clone();
    let bg_drawing = terminal.bg_drawing.clone();

    monitor.connect_changed(move |_, _, _, event| {
        if !matches!(event, gio::FileMonitorEvent::Changed | gio::FileMonitorEvent::Created) {
            return;
        }

        let config = match CustermConfig::load() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[custerm] config reload error: {}", e);
                return;
            }
        };

        eprintln!("[custerm] config reloaded");

        // Font
        let font_desc = gtk4::pango::FontDescription::from_string(
            &format!("{} {}", config.terminal.font_family, config.terminal.font_size),
        );
        term.set_font(Some(&font_desc));

        // Tint
        tint_opacity.set(config.background.tint);
        tint_color.set(crate::terminal::parse_color_pub(&config.background.tint_color));
        tint_overlay.queue_draw();

        // Image opacity
        image_opacity.set(config.background.opacity);
        if has_bg.get() {
            bg_drawing.queue_draw();
        }

        // Background image
        match &config.background.image {
            Some(image) => {
                let path = std::path::Path::new(image);
                if path.exists() {
                    let file = gio::File::for_path(path);
                    if let Ok(texture) = gtk4::gdk::Texture::from_file(&file) {
                        bg_texture.set(Some(texture));
                        bg_drawing.set_visible(true);
                        bg_drawing.queue_draw();
                        tint_overlay.set_visible(true);
                        has_bg.set(true);
                        term.set_clear_background(false);
                        let bg = gtk4::gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
                        term.set_color_background(&bg);
                    }
                }
            }
            None => {
                if has_bg.get() {
                    bg_drawing.set_visible(false);
                    tint_overlay.set_visible(false);
                    has_bg.set(false);
                    term.set_clear_background(true);
                    let bg = crate::terminal::parse_color_pub("#1e1e2e");
                    term.set_color_background(&bg);
                }
            }
        }
    });

    // Keep monitor alive by leaking it (it's for the lifetime of the process)
    std::mem::forget(monitor);
}
