use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "custerm", about = "custerm CLI")]
pub struct Cli {
    /// Socket path override
    #[arg(long)]
    pub socket: Option<String>,

    /// Output JSON format
    #[arg(long, default_value_t = false)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Ping the running custerm instance
    Ping,

    /// Window management
    #[command(subcommand)]
    Window(WindowCommand),

    /// Workspace management
    #[command(subcommand)]
    Workspace(WorkspaceCommand),

    /// Session/surface management
    #[command(subcommand)]
    Session(SessionCommand),

    /// Background image management
    #[command(subcommand)]
    Background(BackgroundCommand),

    /// Tab management
    #[command(subcommand)]
    Tab(TabCommand),

    /// Split pane management
    #[command(subcommand)]
    Split(SplitCommand),

    /// Event stream
    #[command(subcommand)]
    Event(EventCommand),
}

#[derive(Subcommand)]
pub enum WindowCommand {
    /// List all windows
    List,
    /// Create a new window
    New,
    /// Focus a window
    Focus { id: String },
    /// Close a window
    Close { id: Option<String> },
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// List workspaces
    List,
    /// Create a new workspace
    New {
        #[arg(long)]
        name: Option<String>,
    },
    /// Select/switch to a workspace
    Select { id: String },
    /// Close a workspace
    Close { id: Option<String> },
    /// Rename a workspace
    Rename { id: String, name: String },
}

#[derive(Subcommand)]
pub enum SessionCommand {
    /// List all panels
    List,
    /// Get detailed info for a panel
    Info {
        /// Panel ID
        id: String,
    },
    /// Send text to a session
    Send {
        #[arg(long)]
        id: String,
        text: String,
    },
    /// Read screen content (not yet implemented)
    Read {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 50)]
        lines: u32,
    },
    /// Close a session
    Close { id: String },
}

#[derive(Subcommand)]
pub enum BackgroundCommand {
    /// Set background image
    Set { path: String },
    /// Clear background image
    Clear,
    /// Set tint opacity (0.0 - 1.0)
    SetTint { opacity: f64 },
    /// Switch to next random background
    Next,
    /// Toggle background visibility
    Toggle,
}

#[derive(Subcommand)]
pub enum TabCommand {
    /// Create a new tab
    New,
    /// Close the focused tab/panel
    Close,
    /// List tabs
    List,
    /// Extended tab info with panel counts
    Info,
}

#[derive(Subcommand)]
pub enum SplitCommand {
    /// Split horizontally
    Horizontal,
    /// Split vertically
    Vertical,
}

#[derive(Subcommand)]
pub enum EventCommand {
    /// Subscribe to terminal events (streams JSON lines)
    Subscribe,
}

impl Cli {
    pub fn method(&self) -> String {
        match &self.command {
            Command::Ping => "system.ping".to_string(),
            Command::Window(cmd) => match cmd {
                WindowCommand::List => "window.list",
                WindowCommand::New => "window.create",
                WindowCommand::Focus { .. } => "window.focus",
                WindowCommand::Close { .. } => "window.close",
            }
            .to_string(),
            Command::Workspace(cmd) => match cmd {
                WorkspaceCommand::List => "workspace.list",
                WorkspaceCommand::New { .. } => "workspace.create",
                WorkspaceCommand::Select { .. } => "workspace.select",
                WorkspaceCommand::Close { .. } => "workspace.close",
                WorkspaceCommand::Rename { .. } => "workspace.rename",
            }
            .to_string(),
            Command::Session(cmd) => match cmd {
                SessionCommand::List => "session.list",
                SessionCommand::Info { .. } => "session.info",
                SessionCommand::Send { .. } => "session.send_text",
                SessionCommand::Read { .. } => "session.read_text",
                SessionCommand::Close { .. } => "session.close",
            }
            .to_string(),
            Command::Background(cmd) => match cmd {
                BackgroundCommand::Set { .. } => "background.set",
                BackgroundCommand::Clear => "background.clear",
                BackgroundCommand::SetTint { .. } => "background.set_tint",
                BackgroundCommand::Next => "background.next",
                BackgroundCommand::Toggle => "background.toggle",
            }
            .to_string(),
            Command::Tab(cmd) => match cmd {
                TabCommand::New => "tab.new",
                TabCommand::Close => "tab.close",
                TabCommand::List => "tab.list",
                TabCommand::Info => "tab.info",
            }
            .to_string(),
            Command::Split(cmd) => match cmd {
                SplitCommand::Horizontal => "split.horizontal",
                SplitCommand::Vertical => "split.vertical",
            }
            .to_string(),
            Command::Event(cmd) => match cmd {
                EventCommand::Subscribe => "event.subscribe",
            }
            .to_string(),
        }
    }

    pub fn params(&self) -> serde_json::Value {
        match &self.command {
            Command::Ping => json!({}),
            Command::Window(cmd) => match cmd {
                WindowCommand::List | WindowCommand::New => json!({}),
                WindowCommand::Focus { id } => json!({ "window_id": id }),
                WindowCommand::Close { id } => json!({ "window_id": id }),
            },
            Command::Workspace(cmd) => match cmd {
                WorkspaceCommand::List => json!({}),
                WorkspaceCommand::New { name } => json!({ "name": name }),
                WorkspaceCommand::Select { id } => json!({ "workspace_id": id }),
                WorkspaceCommand::Close { id } => json!({ "workspace_id": id }),
                WorkspaceCommand::Rename { id, name } => {
                    json!({ "workspace_id": id, "name": name })
                }
            },
            Command::Session(cmd) => match cmd {
                SessionCommand::List => json!({}),
                SessionCommand::Info { id } => json!({ "id": id }),
                SessionCommand::Send { id, text } => json!({ "session_id": id, "text": text }),
                SessionCommand::Read { id, lines } => json!({ "session_id": id, "lines": lines }),
                SessionCommand::Close { id } => json!({ "session_id": id }),
            },
            Command::Background(cmd) => match cmd {
                BackgroundCommand::Set { path } => {
                    let abs = std::path::Path::new(path)
                        .canonicalize()
                        .unwrap_or_else(|_| std::path::PathBuf::from(path));
                    json!({ "path": abs.to_string_lossy() })
                }
                BackgroundCommand::Clear => json!({}),
                BackgroundCommand::SetTint { opacity } => json!({ "opacity": opacity }),
                BackgroundCommand::Next | BackgroundCommand::Toggle => json!({}),
            },
            Command::Tab(_) | Command::Split(_) | Command::Event(_) => json!({}),
        }
    }
}
