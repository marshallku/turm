mod app;
mod background;
mod panel;
mod plugin_panel;
mod search;
mod service_supervisor;

mod socket;
mod split;
mod statusbar;
mod tabs;
mod terminal;
mod trigger_sink;
mod webview;
mod window;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("turm {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    if args.iter().any(|a| a == "--init-config") {
        match turm_core::config::TurmConfig::write_default() {
            Ok(path) => {
                println!("Config written to: {}", path.display());
                return;
            }
            Err(e) => {
                eprintln!("Failed to write config: {e}");
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|a| a == "--config-path") {
        println!("{}", turm_core::config::TurmConfig::config_path().display());
        return;
    }

    app::run();
}
