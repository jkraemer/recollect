//! Recollect: persistent, searchable memory for coding agents.

pub mod config;
pub mod db;
pub mod detect;
pub mod embed;
pub mod error;
pub mod filter;
pub mod memory;
pub mod migrate;
pub mod output;
pub mod search;
pub mod service;
pub mod time;

pub use error::{Error, Result};
