use std::collections::HashMap;
use std::sync::Mutex;

use crate::config::TurmConfig;
use crate::pty::PtySession;

pub struct Workspace {
    pub id: String,
    pub name: String,
    pub sessions: Vec<String>,
    pub focused_session: Option<String>,
}

pub struct AppState {
    pub config: TurmConfig,
    pub sessions: Mutex<HashMap<String, PtySession>>,
    pub workspaces: Mutex<Vec<Workspace>>,
    pub active_workspace: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(config: TurmConfig) -> Self {
        Self {
            config,
            sessions: Mutex::new(HashMap::new()),
            workspaces: Mutex::new(Vec::new()),
            active_workspace: Mutex::new(None),
        }
    }
}
