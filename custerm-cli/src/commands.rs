use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Parser)]
#[command(name = "custermctl", about = "custerm CLI", version)]
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

    /// Panel management
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

    /// WebView panel management
    #[command(subcommand)]
    Webview(WebviewCommand),

    /// Terminal agent commands (read, exec, state)
    #[command(subcommand)]
    Terminal(TerminalCommand),

    /// AI agent commands (approval workflow)
    #[command(subcommand)]
    Agent(AgentCommand),

    /// Check for updates or update custerm
    #[command(subcommand)]
    Update(UpdateCommand),
}

#[derive(Subcommand)]
pub enum UpdateCommand {
    /// Check if a newer version is available
    Check,
    /// Download and install the latest version
    Apply {
        /// Install a specific version (e.g., v0.1.0)
        #[arg(long)]
        version: Option<String>,
    },
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
    /// Toggle tab bar visibility
    ToggleBar,
    /// Rename a tab
    Rename {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// New title
        title: String,
    },
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

#[derive(Subcommand)]
pub enum TerminalCommand {
    /// Read visible terminal screen text
    Read {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
        /// Start row (0-based, for range read)
        #[arg(long)]
        start_row: Option<i64>,
        /// Start column (0-based, for range read)
        #[arg(long)]
        start_col: Option<i64>,
        /// End row (0-based, for range read)
        #[arg(long)]
        end_row: Option<i64>,
        /// End column (0-based, for range read)
        #[arg(long)]
        end_col: Option<i64>,
    },
    /// Get terminal state (cursor, dimensions, CWD, title)
    State {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
    },
    /// Execute a command in the terminal (sends text + newline)
    Exec {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
        /// Command to execute
        command: String,
    },
    /// Send raw text to the terminal (no newline appended)
    Feed {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
        /// Text to send
        text: String,
    },
    /// Read terminal scrollback history
    History {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
        /// Number of scrollback lines to read
        #[arg(long, default_value_t = 100)]
        lines: i64,
    },
    /// Get combined terminal context (state + screen + scrollback)
    Context {
        /// Panel ID (defaults to active terminal)
        #[arg(long)]
        id: Option<String>,
        /// Number of scrollback history lines to include
        #[arg(long, default_value_t = 50)]
        history_lines: i64,
    },
}

#[derive(Subcommand)]
pub enum AgentCommand {
    /// Request user approval for an action (shows dialog, blocks until response)
    Approve {
        /// Dialog message describing the action
        message: String,
        /// Dialog title
        #[arg(long, default_value = "Agent Action")]
        title: String,
        /// Custom button labels (comma-separated, first = approve)
        #[arg(long)]
        actions: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum WebviewCommand {
    /// Open a URL in a new webview panel
    Open {
        /// URL to open
        url: String,
        /// Panel mode: tab, split_h, split_v
        #[arg(long, default_value = "tab")]
        mode: String,
    },
    /// Navigate an existing webview to a new URL
    Navigate {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// URL to navigate to
        url: String,
    },
    /// Go back in webview history
    Back {
        /// Panel ID
        #[arg(long)]
        id: String,
    },
    /// Go forward in webview history
    Forward {
        /// Panel ID
        #[arg(long)]
        id: String,
    },
    /// Reload webview
    Reload {
        /// Panel ID
        #[arg(long)]
        id: String,
    },
    /// Execute JavaScript in a webview
    ExecJs {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// JavaScript code to execute
        code: String,
    },
    /// Get page content from a webview
    GetContent {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// Content format: text or html
        #[arg(long, default_value = "text")]
        format: String,
    },
    /// Take a screenshot of a webview (returns base64 PNG or saves to file)
    Screenshot {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// Save to file path (omit for base64 in response)
        #[arg(long)]
        path: Option<String>,
    },
    /// Query a single DOM element by CSS selector
    Query {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector
        selector: String,
    },
    /// Query all matching DOM elements by CSS selector
    QueryAll {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector
        selector: String,
        /// Max results
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Get computed CSS styles for an element
    GetStyles {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector
        selector: String,
        /// CSS property names (comma-separated)
        properties: String,
    },
    /// Click a DOM element by CSS selector
    Click {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector
        selector: String,
    },
    /// Type text into an input element
    Fill {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector for the input element
        selector: String,
        /// Value to type
        value: String,
    },
    /// Scroll to position or element
    Scroll {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// CSS selector to scroll to (overrides x/y)
        #[arg(long)]
        selector: Option<String>,
        /// X scroll position
        #[arg(long, default_value_t = 0)]
        x: i32,
        /// Y scroll position
        #[arg(long, default_value_t = 0)]
        y: i32,
    },
    /// Get page metadata (title, dimensions, element counts)
    PageInfo {
        /// Panel ID
        #[arg(long)]
        id: String,
    },
    /// Toggle DevTools inspector
    Devtools {
        /// Panel ID
        #[arg(long)]
        id: String,
        /// Action: show, close, attach, detach
        #[arg(default_value = "show")]
        action: String,
    },
}

impl Cli {
    pub fn method(&self) -> String {
        match &self.command {
            Command::Ping => "system.ping".to_string(),
            Command::Session(cmd) => match cmd {
                SessionCommand::List => "session.list",
                SessionCommand::Info { .. } => "session.info",
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
                TabCommand::ToggleBar => "tabs.toggle_bar",
                TabCommand::Rename { .. } => "tab.rename",
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
            Command::Webview(cmd) => match cmd {
                WebviewCommand::Open { .. } => "webview.open",
                WebviewCommand::Navigate { .. } => "webview.navigate",
                WebviewCommand::Back { .. } => "webview.back",
                WebviewCommand::Forward { .. } => "webview.forward",
                WebviewCommand::Reload { .. } => "webview.reload",
                WebviewCommand::ExecJs { .. } => "webview.execute_js",
                WebviewCommand::GetContent { .. } => "webview.get_content",
                WebviewCommand::Screenshot { .. } => "webview.screenshot",
                WebviewCommand::Query { .. } => "webview.query",
                WebviewCommand::QueryAll { .. } => "webview.query_all",
                WebviewCommand::GetStyles { .. } => "webview.get_styles",
                WebviewCommand::Click { .. } => "webview.click",
                WebviewCommand::Fill { .. } => "webview.fill",
                WebviewCommand::Scroll { .. } => "webview.scroll",
                WebviewCommand::PageInfo { .. } => "webview.page_info",
                WebviewCommand::Devtools { .. } => "webview.devtools",
            }
            .to_string(),
            Command::Terminal(cmd) => match cmd {
                TerminalCommand::Read { .. } => "terminal.read",
                TerminalCommand::State { .. } => "terminal.state",
                TerminalCommand::Exec { .. } => "terminal.exec",
                TerminalCommand::Feed { .. } => "terminal.feed",
                TerminalCommand::History { .. } => "terminal.history",
                TerminalCommand::Context { .. } => "terminal.context",
            }
            .to_string(),
            Command::Agent(cmd) => match cmd {
                AgentCommand::Approve { .. } => "agent.approve",
            }
            .to_string(),
            Command::Update(_) => unreachable!("update commands are handled locally"),
        }
    }

    pub fn params(&self) -> serde_json::Value {
        match &self.command {
            Command::Ping => json!({}),
            Command::Session(cmd) => match cmd {
                SessionCommand::List => json!({}),
                SessionCommand::Info { id } => json!({ "id": id }),
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
            Command::Tab(cmd) => match cmd {
                TabCommand::Rename { id, title } => json!({ "id": id, "title": title }),
                _ => json!({}),
            },
            Command::Terminal(cmd) => match cmd {
                TerminalCommand::Read {
                    id,
                    start_row,
                    start_col,
                    end_row,
                    end_col,
                } => {
                    let mut p = json!({});
                    if let Some(id) = id {
                        p["id"] = json!(id);
                    }
                    if let Some(sr) = start_row {
                        p["start_row"] = json!(sr);
                        p["start_col"] = json!(start_col.unwrap_or(0));
                        p["end_row"] = json!(end_row.unwrap_or(*sr));
                        p["end_col"] = json!(end_col.unwrap_or(999));
                    }
                    p
                }
                TerminalCommand::State { id } => match id {
                    Some(id) => json!({ "id": id }),
                    None => json!({}),
                },
                TerminalCommand::Exec { id, command } => match id {
                    Some(id) => json!({ "id": id, "command": command }),
                    None => json!({ "command": command }),
                },
                TerminalCommand::Feed { id, text } => match id {
                    Some(id) => json!({ "id": id, "text": text }),
                    None => json!({ "text": text }),
                },
                TerminalCommand::History { id, lines } => {
                    let mut p = json!({ "lines": lines });
                    if let Some(id) = id {
                        p["id"] = json!(id);
                    }
                    p
                }
                TerminalCommand::Context { id, history_lines } => {
                    let mut p = json!({ "history_lines": history_lines });
                    if let Some(id) = id {
                        p["id"] = json!(id);
                    }
                    p
                }
            },
            Command::Agent(cmd) => match cmd {
                AgentCommand::Approve {
                    message,
                    title,
                    actions,
                } => {
                    let mut p = json!({ "message": message, "title": title });
                    if let Some(actions) = actions {
                        let acts: Vec<&str> = actions.split(',').map(|s| s.trim()).collect();
                        p["actions"] = json!(acts);
                    }
                    p
                }
            },
            Command::Split(_) | Command::Event(_) | Command::Update(_) => {
                json!({})
            }
            Command::Webview(cmd) => match cmd {
                WebviewCommand::Open { url, mode } => json!({ "url": url, "mode": mode }),
                WebviewCommand::Navigate { id, url } => json!({ "id": id, "url": url }),
                WebviewCommand::Back { id } => json!({ "id": id }),
                WebviewCommand::Forward { id } => json!({ "id": id }),
                WebviewCommand::Reload { id } => json!({ "id": id }),
                WebviewCommand::ExecJs { id, code } => json!({ "id": id, "code": code }),
                WebviewCommand::GetContent { id, format } => json!({ "id": id, "format": format }),
                WebviewCommand::Screenshot { id, path } => json!({ "id": id, "path": path }),
                WebviewCommand::Query { id, selector } => json!({ "id": id, "selector": selector }),
                WebviewCommand::QueryAll {
                    id,
                    selector,
                    limit,
                } => json!({ "id": id, "selector": selector, "limit": limit }),
                WebviewCommand::GetStyles {
                    id,
                    selector,
                    properties,
                } => {
                    let props: Vec<&str> = properties.split(',').map(|s| s.trim()).collect();
                    json!({ "id": id, "selector": selector, "properties": props })
                }
                WebviewCommand::Click { id, selector } => json!({ "id": id, "selector": selector }),
                WebviewCommand::Fill {
                    id,
                    selector,
                    value,
                } => json!({ "id": id, "selector": selector, "value": value }),
                WebviewCommand::Scroll { id, selector, x, y } => {
                    json!({ "id": id, "selector": selector, "x": x, "y": y })
                }
                WebviewCommand::PageInfo { id } => json!({ "id": id }),
                WebviewCommand::Devtools { id, action } => json!({ "id": id, "action": action }),
            },
        }
    }
}
