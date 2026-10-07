//! The update check and `recollect update`, end to end against a fake
//! release site on a local port. Nothing here reaches GitHub.

mod common;

use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use common::release::{self, ReleaseServer, SYSTEM_PATH};
use predicates::prelude::*;
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

const AFTER_AN_UPDATE: &str = "A running `recollect serve` switches to the new version by itself.\nIf sync with another machine stops, update recollect there too.\n";

/// The binary under test, installed as `bin/<name>` in a directory of its
/// own, with a data directory beside it.
struct Installation {
    dir: TempDir,
    name: &'static str,
}

impl Installation {
    fn new() -> Self {
        Self::named("recollect")
    }

    fn named(name: &'static str) -> Self {
        let dir = common::target_tempdir();
        std::fs::create_dir(dir.path().join("bin")).unwrap();
        std::fs::create_dir(dir.path().join("data")).unwrap();
        common::install_binary(&dir.path().join("bin").join(name));
        Self { dir, name }
    }

    /// The directory the binary is installed in, as the binary itself sees it.
    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin").canonicalize().unwrap()
    }

    fn binary(&self) -> PathBuf {
        self.bin().join(self.name)
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// The installed binary, started as `program`, with releases at `base`
    /// and its own directory first on PATH, as on a set-up machine.
    fn run(&self, program: &Path, base: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("RECOLLECT_DATA_DIR", self.data())
            .env("RECOLLECT_DOWNLOAD_BASE", base)
            .env("PATH", format!("{}:{SYSTEM_PATH}", self.bin().display()));
        command
    }

    fn update(&self, base: &str) -> Command {
        let mut command = self.run(&self.binary(), base);
        command.arg("update");
        command
    }

    /// What the installed binary says its version is.
    fn version(&self) -> String {
        let output = StdCommand::new(self.binary())
            .arg("--version")
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap()
    }

    fn remembered(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.data().join(CHECK_FILE)).unwrap())
            .unwrap()
    }
}

/// A release site whose latest release is 99.0.0, a stub that says so.
fn site_with_release_99() -> (TempDir, ReleaseServer) {
    let root = tempfile::tempdir().unwrap();
    release::publish(&root.path().join("download/v99.0.0"), |_| {
        "echo \"recollect 99.0.0\"".to_string()
    });
    let server = ReleaseServer::start(root.path(), Some("v99.0.0"));
    (root, server)
}

#[test]
fn update_installs_the_latest_release_over_the_running_binary() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    installation
        .update(&site.base)
        .assert()
        .success()
        .stdout(format!(
            "installed recollect 99.0.0 to {}\n{AFTER_AN_UPDATE}",
            installation.binary().display()
        ))
        .stderr("");
    assert_eq!(installation.version(), "recollect 99.0.0\n");
    let requests = site.requests();
    assert_eq!(requests.len(), 3, "{requests:?}");
    assert_eq!(requests[0], "/latest");
    assert!(
        requests[1].starts_with("/download/v99.0.0/recollect-"),
        "{requests:?}"
    );
    assert_eq!(requests[2], "/download/v99.0.0/SHA256SUMS");
    assert_eq!(installation.remembered()["latest"], "99.0.0");
    let left: Vec<_> = std::fs::read_dir(installation.bin())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["recollect"], "nothing but the binary is left");
}

#[test]
fn update_works_through_a_symlink_to_the_binary() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    let link = installation.dir.path().join("rc");
    std::os::unix::fs::symlink(installation.binary(), &link).unwrap();
    installation
        .run(&link, &site.base)
        .arg("update")
        .assert()
        .success()
        .stderr("");
    assert_eq!(installation.version(), "recollect 99.0.0\n");
}

#[test]
fn update_with_nothing_newer_downloads_nothing() {
    for tag in [format!("v{INSTALLED}"), "v0.0.1".to_string()] {
        let installation = Installation::new();
        let (_root, site) = site_with_latest(&tag);
        installation
            .update(&site.base)
            .assert()
            .success()
            .stdout(format!("recollect {INSTALLED} is the latest release\n"))
            .stderr("");
        assert_eq!(site.requests(), ["/latest"], "{tag}");
        assert_eq!(installation.version(), format!("recollect {INSTALLED}\n"));
    }
}

#[test]
fn update_check_says_whether_there_is_a_newer_release_and_installs_nothing() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    installation
        .update(&site.base)
        .arg("--check")
        .assert()
        .success()
        .stdout(format!(
            "recollect 99.0.0 is available (installed: {INSTALLED})\n"
        ))
        .stderr("");
    assert_eq!(site.requests(), ["/latest"]);
    assert_eq!(installation.version(), format!("recollect {INSTALLED}\n"));
    assert_eq!(installation.remembered()["latest"], "99.0.0");

    let (_root, current) = site_with_latest(&format!("v{INSTALLED}"));
    installation
        .update(&current.base)
        .arg("--check")
        .assert()
        .success()
        .stdout(format!("recollect {INSTALLED} is the latest release\n"))
        .stderr("");
}

#[test]
fn update_check_ignores_the_switch_for_the_session_notice() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    std::fs::write(
        installation.data().join("config.toml"),
        "[update]\ncheck = false\n",
    )
    .unwrap();
    installation
        .update(&site.base)
        .arg("--check")
        .assert()
        .success()
        .stdout(format!(
            "recollect 99.0.0 is available (installed: {INSTALLED})\n"
        ));
}

#[test]
fn update_does_not_need_the_database() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    std::fs::write(installation.data().join("memories.db"), "not a database").unwrap();
    installation
        .update(&site.base)
        .assert()
        .success()
        .stderr("");
    assert_eq!(installation.version(), "recollect 99.0.0\n");
}

#[test]
fn update_without_an_answer_fails_and_says_why() {
    let installation = Installation::new();
    let empty = tempfile::tempdir().unwrap();
    let site = ReleaseServer::start(empty.path(), None);
    for args in [&["update"][..], &["update", "--check"]] {
        installation
            .run(&installation.binary(), &site.base)
            .args(args)
            .assert()
            .code(1)
            .stdout("")
            .stderr(format!(
                "error: could not look up the latest release at {}/latest: no release in the answer (HTTP 404)\n",
                site.base
            ));
    }
    assert!(!installation.data().join(CHECK_FILE).exists());
}

#[test]
fn a_development_build_updates_only_from_a_named_release_site() {
    let data = tempfile::tempdir().unwrap();
    for args in [&["update"][..], &["update", "--check"]] {
        recollect(data.path())
            .args(args)
            .assert()
            .code(1)
            .stdout("")
            .stderr(
                "error: this is a development build; set RECOLLECT_DOWNLOAD_BASE to the releases it may look up and install\n",
            );
    }
}

#[test]
fn update_refuses_a_binary_that_is_not_named_recollect() {
    let installation = Installation::named("recollect-dev");
    let (_root, site) = site_with_release_99();
    installation
        .update(&site.base)
        .assert()
        .code(1)
        .stdout("")
        .stderr(format!(
            "error: {} is not named recollect, so the installer would not replace it; install with install.sh instead\n",
            installation.binary().display()
        ));
    assert_eq!(site.requests(), ["/latest"], "nothing was downloaded");
}

#[test]
fn update_refuses_a_directory_it_cannot_write_to() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    let read_only = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(installation.bin(), read_only).unwrap();
    let outcome = installation.update(&site.base).assert();
    // Writable again before anything can fail, so the directory can be removed.
    std::fs::set_permissions(installation.bin(), std::fs::Permissions::from_mode(0o755)).unwrap();
    outcome
        .code(1)
        .stdout("")
        .stderr(predicate::str::starts_with(format!(
            "error: cannot write to {}: ",
            installation.bin().display()
        )));
    assert_eq!(site.requests(), ["/latest"], "nothing was downloaded");
    assert_eq!(installation.version(), format!("recollect {INSTALLED}\n"));
}

#[test]
fn a_failed_install_keeps_the_running_binary() {
    let installation = Installation::new();
    let (root, site) = site_with_release_99();
    let sums = root.path().join("download/v99.0.0/SHA256SUMS");
    let wrong: String = std::fs::read_to_string(&sums)
        .unwrap()
        .lines()
        .map(|line| format!("{}  {}\n", "0".repeat(64), line.split_once("  ").unwrap().1))
        .collect();
    std::fs::write(&sums, wrong).unwrap();
    installation
        .update(&site.base)
        .assert()
        .code(1)
        .stdout("")
        .stderr(
            predicate::str::is_match("^error: checksum mismatch for recollect-[^\n]+\n$").unwrap(),
        );
    assert_eq!(installation.version(), format!("recollect {INSTALLED}\n"));
}

#[test]
fn update_exits_with_the_status_of_the_installer() {
    let installation = Installation::new();
    let (_root, site) = site_with_release_99();
    let stubs = tempfile::tempdir().unwrap();
    release::write_script(&stubs.path().join("tar"), "exit 7");
    installation
        .update(&site.base)
        .env(
            "PATH",
            format!(
                "{}:{}:{SYSTEM_PATH}",
                stubs.path().display(),
                installation.bin().display()
            ),
        )
        .assert()
        .code(7);
    assert_eq!(installation.version(), format!("recollect {INSTALLED}\n"));
}
