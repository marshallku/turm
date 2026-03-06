use gtk4::prelude::*;
use gtk4::Application;

use crate::window::CustermWindow;

const APP_ID: &str = "com.marshall.custerm";

pub fn run() {
    let app = Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_startup(|_| {
        // Force dark theme
        if let Some(settings) = gtk4::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }
    });

    app.connect_activate(|app| {
        let config = custerm_core::config::CustermConfig::load()
            .unwrap_or_default();
        let window = CustermWindow::new(app, &config);
        window.present();
    });

    app.run();
}
