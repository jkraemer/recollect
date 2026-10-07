//! The update check and `recollect update`, end to end against a fake
//! release site on a local port. Nothing here reaches GitHub.

mod common;

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use common::release::ReleaseServer;
use recollect::time::now_timestamp;
use recollect::update::CHECK_FILE;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The version of the binary under test.
const INSTALLED: &str = env!("CARGO_PKG_VERSION");

/// `recollect` against `data_dir`, with the model already downloaded so no
/// download notice appears on stderr, and no release site unless a test
/// names one.
fn recollect(data_dir: &Path) -> Command {
    let _ = common::shared_model();
    let mut command = Command::cargo_bin("recollect").unwrap();
    command
        .env("RECOLLECT_DATA_DIR", data_dir)
        .env("RECOLLECT_MODEL_DIR", common::model_dir())
        .env_remove("RECOLLECT_DOWNLOAD_BASE");
    command
}

/// The line a session is told about release `latest`.
fn notice(latest: &str) -> String {
    format!(
        "recollect {latest} is available (installed: {INSTALLED}). Tell the user they can run `recollect update`; do not run it yourself.\n"
    )
}

/// A release site without files whose latest release is `tag`.
fn site_with_latest(tag: &str) -> (TempDir, ReleaseServer) {
    let root = tempfile::tempdir().unwrap();
    let server = ReleaseServer::start(root.path(), Some(tag));
    (root, server)
}

/// A machine's data directory and a directory outside any repository in
/// which its sessions start.
struct Session {
    data: TempDir,
    cwd: TempDir,
}

impl Session {
    fn new() -> Self {
        Self {
            data: tempfile::tempdir().unwrap(),
            cwd: tempfile::tempdir().unwrap(),
        }
    }

    /// What `hook session-start` prints for a session started by `source`,
    /// with releases looked up at `base`. It must succeed without a word on
    /// stderr.
    fn start(&self, base: Option<&str>, source: &str) -> String {
        let mut command = recollect(self.data.path());
        command
            .args(["hook", "session-start"])
            .write_stdin(json!({"cwd": self.cwd.path(), "source": source}).to_string());
        if let Some(base) = base {
            command.env("RECOLLECT_DOWNLOAD_BASE", base);
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(stderr, "");
        String::from_utf8(output.stdout).unwrap()
    }

    fn check_file(&self) -> PathBuf {
        self.data.path().join(CHECK_FILE)
    }

    /// Makes the machine remember a lookup at `checked_at` that found `latest`.
    fn remember(&self, checked_at: &str, latest: &str) {
        std::fs::write(
            self.check_file(),
            json!({"checked_at": checked_at, "latest": latest}).to_string(),
        )
        .unwrap();
    }

    fn remembered(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.check_file()).unwrap()).unwrap()
    }
}

#[test]
fn a_session_starts_with_a_notice_about_a_newer_release() {
    let session = Session::new();
    let (_root, site) = site_with_latest("v99.0.0");
    let without = session.start(None, "startup");
    let with = session.start(Some(&site.base), "startup");
    assert_eq!(with, format!("{without}\n{}", notice("99.0.0")));
    assert_eq!(site.requests(), ["/latest"]);
    assert_eq!(session.remembered()["latest"], "99.0.0");
}

#[test]
fn a_session_started_by_clear_gets_the_notice_too() {
    let session = Session::new();
    let (_root, site) = site_with_latest("v99.0.0");
    let stdout = session.start(Some(&site.base), "clear");
    assert!(
        stdout.ends_with(&format!("\n\n{}", notice("99.0.0"))),
        "{stdout}"
    );
}

#[test]
fn no_notice_when_the_latest_release_is_installed_or_older() {
    for tag in [format!("v{INSTALLED}"), "v0.0.1".to_string()] {
        let session = Session::new();
        let (_root, site) = site_with_latest(&tag);
        let without = session.start(None, "startup");
        assert_eq!(session.start(Some(&site.base), "startup"), without, "{tag}");
        assert_eq!(site.requests(), ["/latest"], "{tag}");
    }
}

#[test]
fn the_latest_release_is_looked_up_once_a_day() {
    let session = Session::new();
    let (_root, site) = site_with_latest("v99.0.0");
    let first = session.start(Some(&site.base), "startup");
    let second = session.start(Some(&site.base), "startup");
    assert_eq!(second, first, "the notice is repeated from the check file");
    assert_eq!(site.requests(), ["/latest"]);

    session.remember("2026-01-01T00:00:00.000Z", "0.0.1");
    let later = session.start(Some(&site.base), "startup");
    assert_eq!(later, first);
    assert_eq!(site.requests(), ["/latest", "/latest"]);
}

#[test]
fn after_a_compaction_nothing_is_looked_up_and_nothing_is_said() {
    let session = Session::new();
    let (_root, site) = site_with_latest("v99.0.0");
    session.remember(&now_timestamp(), "99.0.0");
    let without = session.start(None, "compact");
    assert_eq!(session.start(Some(&site.base), "compact"), without);
    assert_eq!(site.requests(), Vec::<String>::new());
}

#[test]
fn the_check_can_be_switched_off_in_the_config() {
    let session = Session::new();
    let (_root, site) = site_with_latest("v99.0.0");
    let without = session.start(None, "startup");
    std::fs::write(
        session.data.path().join("config.toml"),
        "[update]\ncheck = false\n",
    )
    .unwrap();
    // Neither looked up nor repeated from an earlier lookup.
    session.remember("2026-01-01T00:00:00.000Z", "99.0.0");
    assert_eq!(session.start(Some(&site.base), "startup"), without);
    assert_eq!(site.requests(), Vec::<String>::new());
}

#[test]
fn a_lookup_without_an_answer_leaves_the_session_start_as_it_is() {
    let empty = tempfile::tempdir().unwrap();
    let not_found = ReleaseServer::start(empty.path(), None);
    // Nothing listens on the port of a listener that is gone.
    let closed = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}", listener.local_addr().unwrap())
    };
    // Connections are accepted by the system and never read.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let never_answers = format!("http://{}", silent.local_addr().unwrap());

    for base in [not_found.base.as_str(), &closed, &never_answers] {
        let session = Session::new();
        let without = session.start(None, "startup");
        let started = Instant::now();
        assert_eq!(session.start(Some(base), "startup"), without, "{base}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "{base} held the session start up for {:?}",
            started.elapsed()
        );
        assert!(
            session.remembered()["checked_at"].is_string(),
            "the attempt is remembered: {base}"
        );
        assert!(session.remembered()["latest"].is_null(), "{base}");
    }
}
