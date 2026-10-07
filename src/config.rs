//! Where recollect keeps its data and how search is tuned.
//!
//! The data directory comes from `RECOLLECT_DATA_DIR` (default `~/.recollect`)
//! and may hold an optional `config.toml`. The embedding model cache comes from
//! `RECOLLECT_MODEL_DIR` (default `<data dir>/models`).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

pub const DATABASE_FILE: &str = "memories.db";
pub const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RecencyConfig {
    /// 0 disables recency ranking, 1 applies the full decay.
    pub aging_factor: f64,
    pub half_life_days: f64,
}

impl Default for RecencyConfig {
    fn default() -> Self {
        Self {
            aging_factor: 0.0,
            half_life_days: 30.0,
        }
    }
}

/// The `[sync]` table: how this machine takes part in sync.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SyncConfig {
    /// The address and port `recollect serve` listens on.
    pub listen: SocketAddr,
    /// Seconds between the daemon's rounds with its peers.
    pub interval_seconds: u64,
    /// This machine's name as its peers store it; its short host name when unset.
    pub name: Option<String>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            // 7326 is the Ruby server's port.
            listen: SocketAddr::from(([0, 0, 0, 0], 7327)),
            interval_seconds: 300,
            name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub data_dir: PathBuf,
    pub model_dir: PathBuf,
    /// Cosine distance beyond which a chunk does not count as a vector match.
    pub max_vector_distance: f64,
    pub recency: RecencyConfig,
    pub sync: SyncConfig,
}

impl Config {
    /// Resolves directories from the environment, then reads `config.toml`.
    pub fn load() -> Result<Self> {
        let data_dir = match non_empty_env("RECOLLECT_DATA_DIR") {
            Some(dir) => dir,
            None => home_dir()?.join(".recollect"),
        };
        Self::load_from(data_dir, non_empty_env("RECOLLECT_MODEL_DIR"))
    }

    pub fn load_from(data_dir: PathBuf, model_dir: Option<PathBuf>) -> Result<Self> {
        let path = data_dir.join(CONFIG_FILE);
        let file = read_file_config(&path)?;
        let invalid = |message: &str| Error::Config {
            path: path.display().to_string(),
            message: message.to_string(),
        };
        if !(0.0..=1.0).contains(&file.recency.aging_factor) {
            return Err(invalid("recency.aging_factor must be between 0 and 1"));
        }
        if !(file.recency.half_life_days.is_finite() && file.recency.half_life_days > 0.0) {
            return Err(invalid(
                "recency.half_life_days must be finite and positive",
            ));
        }
        if !(0.0..=2.0).contains(&file.search.max_vector_distance) {
            return Err(invalid(
                "search.max_vector_distance must be between 0 and 2",
            ));
        }
        if file.sync.interval_seconds == 0 {
            return Err(invalid("sync.interval_seconds must be at least 1"));
        }
        if let Some(name) = &file.sync.name
            && !crate::sync::is_valid_peer_name(name)
        {
            return Err(invalid(
                "sync.name may only contain letters, digits, '.', '_' and '-' (at most 64)",
            ));
        }
        Ok(Self {
            model_dir: model_dir.unwrap_or_else(|| data_dir.join("models")),
            data_dir,
            max_vector_distance: file.search.max_vector_distance,
            recency: file.recency,
            sync: file.sync,
        })
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE)
    }
}

/// `config.toml`; every missing section and key takes its default.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    search: SearchSection,
    recency: RecencyConfig,
    sync: SyncConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SearchSection {
    max_vector_distance: f64,
}

impl Default for SearchSection {
    fn default() -> Self {
        Self {
            // bge-small-en-v1.5 rates even unrelated text as fairly similar; at
            // this distance off-topic queries get no vector matches on real
            // memories while on-topic queries still reach their relevant ones.
            max_vector_distance: 0.375,
        }
    }
}

fn read_file_config(path: &Path) -> Result<FileConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|err| Error::Config {
            path: path.display().to_string(),
            message: err.message().to_string(),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(err) => Err(Error::file(path)(err)),
    }
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn home_dir() -> Result<PathBuf> {
    non_empty_env("HOME").ok_or(Error::NoDataDir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, text: &str) {
        std::fs::write(dir.join(CONFIG_FILE), text).unwrap();
    }

    #[test]
    fn defaults_apply_without_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(dir.path().to_path_buf(), None).unwrap();
        assert_eq!(config.model_dir, dir.path().join("models"));
        assert_eq!(config.database_path(), dir.path().join("memories.db"));
        assert_eq!(config.max_vector_distance, 0.375);
        assert_eq!(
            config.recency,
            RecencyConfig {
                aging_factor: 0.0,
                half_life_days: 30.0
            }
        );
    }

    #[test]
    fn an_explicit_model_dir_wins() {
        let dir = tempfile::tempdir().unwrap();
        let models = dir.path().join("elsewhere");
        let config = Config::load_from(dir.path().to_path_buf(), Some(models.clone())).unwrap();
        assert_eq!(config.model_dir, models);
    }

    #[test]
    fn values_come_from_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "[search]\nmax_vector_distance = 0.4\n\n[recency]\naging_factor = 0.5\nhalf_life_days = 7.0\n",
        );
        let config = Config::load_from(dir.path().to_path_buf(), None).unwrap();
        assert_eq!(config.max_vector_distance, 0.4);
        assert_eq!(
            config.recency,
            RecencyConfig {
                aging_factor: 0.5,
                half_life_days: 7.0
            }
        );
    }

    #[test]
    fn unknown_keys_are_rejected_so_typos_surface() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "[search]\nmax_distance = 0.4\n");
        let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
        assert!(
            matches!(&err, Error::Config { message, .. } if message.contains("max_distance")),
            "{err}"
        );
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        for (text, expected) in [
            (
                "[recency]\naging_factor = 1.5\n",
                "recency.aging_factor must be between 0 and 1",
            ),
            (
                "[recency]\nhalf_life_days = 0.0\n",
                "recency.half_life_days must be finite and positive",
            ),
            (
                "[recency]\nhalf_life_days = nan\n",
                "recency.half_life_days must be finite and positive",
            ),
            (
                "[recency]\nhalf_life_days = inf\n",
                "recency.half_life_days must be finite and positive",
            ),
            (
                "[search]\nmax_vector_distance = 3.0\n",
                "search.max_vector_distance must be between 0 and 2",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_config(dir.path(), text);
            let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
            assert!(
                matches!(&err, Error::Config { message, .. } if message == expected),
                "{err}"
            );
        }
    }

    #[test]
    fn range_bounds_are_inclusive() {
        let dir = tempfile::tempdir().unwrap();
        for text in [
            "[recency]\naging_factor = 0.0\n",
            "[recency]\naging_factor = 1.0\n",
            "[search]\nmax_vector_distance = 0.0\n",
            "[search]\nmax_vector_distance = 2.0\n",
        ] {
            write_config(dir.path(), text);
            assert!(
                Config::load_from(dir.path().to_path_buf(), None).is_ok(),
                "{text}"
            );
        }
    }

    #[test]
    fn malformed_toml_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "[search\n");
        let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
        let expected_path = dir.path().join(CONFIG_FILE).display().to_string();
        assert!(
            matches!(&err, Error::Config { path, .. } if *path == expected_path),
            "{err}"
        );
    }

    #[test]
    fn an_unreadable_config_file_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(CONFIG_FILE)).unwrap();
        let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
        assert!(
            matches!(&err, Error::File { path, .. } if *path == dir.path().join(CONFIG_FILE)),
            "{err}"
        );
    }

    #[test]
    fn sync_listens_on_7327_every_five_minutes_under_the_host_name_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(dir.path().to_path_buf(), None).unwrap();
        assert_eq!(config.sync.listen.to_string(), "0.0.0.0:7327");
        assert_eq!(config.sync.interval_seconds, 300);
        assert_eq!(config.sync.name, None);
    }

    #[test]
    fn sync_settings_come_from_config_toml() {
        let dir = tempfile::tempdir().unwrap();
        write_config(
            dir.path(),
            "[sync]\nlisten = \"127.0.0.1:9000\"\ninterval_seconds = 60\nname = \"laptop\"\n",
        );
        let config = Config::load_from(dir.path().to_path_buf(), None).unwrap();
        assert_eq!(config.sync.listen.to_string(), "127.0.0.1:9000");
        assert_eq!(config.sync.interval_seconds, 60);
        assert_eq!(config.sync.name.as_deref(), Some("laptop"));
    }

    #[test]
    fn invalid_sync_settings_are_rejected() {
        for (text, expected) in [
            (
                "[sync]\ninterval_seconds = 0\n",
                "sync.interval_seconds must be at least 1",
            ),
            (
                "[sync]\nname = \"two words\"\n",
                "sync.name may only contain letters, digits, '.', '_' and '-' (at most 64)",
            ),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_config(dir.path(), text);
            let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
            assert!(
                matches!(&err, Error::Config { message, .. } if message == expected),
                "{err}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), "[sync]\nlisten = \"nowhere\"\n");
        let err = Config::load_from(dir.path().to_path_buf(), None).unwrap_err();
        assert!(
            matches!(&err, Error::Config { message, .. } if message.contains("socket address")),
            "{err}"
        );
    }
}
