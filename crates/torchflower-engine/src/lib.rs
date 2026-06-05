#![allow(unknown_lints)]
#![recursion_limit = "256"]
#![allow(
    clippy::collapsible_if,
    clippy::excessive_precision,
    clippy::field_reassign_with_default,
    clippy::manual_div_ceil,
    clippy::manual_is_multiple_of,
    clippy::needless_borrow,
    clippy::too_many_arguments,
    clippy::vec_init_then_push
)]
pub mod api;
#[path = "bedrock/auth/mod.rs"]
pub mod auth;
pub mod bedrock;
pub mod bot;
pub mod config;
pub mod core;
pub mod db;
pub mod diagnostics;
pub mod error;
pub mod models;
pub mod pool;
pub mod validation;
