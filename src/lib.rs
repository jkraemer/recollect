//! Recollect: persistent, searchable memory for coding agents.

pub mod config;
pub mod db;
pub mod error;
pub mod memory;
pub mod time;

pub use error::{Error, Result};
