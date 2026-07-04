// bole-3xj5
//! Shared server state handed to every handler.

use std::sync::Arc;

use crate::config::AuthConfig;

/// Cloneable application state (cheap: everything behind `Arc`).
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<bole::Repository>,
    pub config: Arc<AuthConfig>,
}
