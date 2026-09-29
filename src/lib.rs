//! Recollect: persistent, searchable memory for coding agents.

pub mod config;
pub mod db;
pub mod embed;
pub mod error;
pub mod filter;
pub mod memory;
pub mod time;

pub use error::{Error, Result};
