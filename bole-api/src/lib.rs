// bole-3xj5
//! bole-api: HTTP/JSON read API over a bole repository.

pub mod config;
pub mod error;
pub mod handlers;
pub mod router;
pub mod state;

pub use router::build_router;
pub use state::AppState;
