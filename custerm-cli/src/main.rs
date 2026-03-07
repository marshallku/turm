mod client;
mod commands;

use clap::Parser;
use commands::{Cli, Command, EventCommand};

fn main() {
    let cli = Cli::parse();

    let socket_path = cli.socket.clone().unwrap_or_else(|| {
        std::env::var("CUSTERM_SOCKET").unwrap_or_else(|_| "/tmp/custerm.sock".to_string())
    });

    // Event subscribe is a long-lived streaming connection
    if matches!(&cli.command, Command::Event(EventCommand::Subscribe)) {
        match client::subscribe(&socket_path) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Failed to subscribe: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let result = client::send_command(&socket_path, &cli.method(), cli.params());

    match result {
        Ok(response) => {
            if response.ok {
                if let Some(result) = response.result {
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    } else {
                        print_result(&result);
                    }
                }
            } else if let Some(err) = response.error {
                eprintln!("Error [{}]: {}", err.code, err.message);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            std::process::exit(1);
        }
    }
}

fn print_result(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => println!("{s}"),
        serde_json::Value::Array(arr) => {
            for item in arr {
                println!("{}", serde_json::to_string(item).unwrap());
            }
        }
        other => println!("{}", serde_json::to_string_pretty(other).unwrap()),
    }
}
