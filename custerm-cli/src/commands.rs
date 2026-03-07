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
            Command::Tab(_) | Command::Split(_) | Command::Event(_) | Command::Update(_) => {
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
