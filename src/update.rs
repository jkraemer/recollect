//! Newer releases: looking up the latest one, remembering the answer between
//! sessions, and the notice a session starts with.

use std::ffi::OsStr;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::time::format_timestamp;

/// Where releases live unless `RECOLLECT_DOWNLOAD_BASE` names another place;
/// install.sh has the same default.
const DEFAULT_DOWNLOAD_BASE: &str = "https://github.com/jkraemer/recollect/releases";

/// The file in the data directory that remembers the last lookup.
pub const CHECK_FILE: &str = "update-check.json";

/// How long a lookup may take in all: it runs while a session starts.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a lookup, or a failed attempt at one, is good for.
const CHECK_INTERVAL: TimeDelta = TimeDelta::hours(24);

/// A release version, `X.Y.Z`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// Exactly three numbers separated by dots; anything else is no version.
    pub fn parse(text: &str) -> Option<Self> {
        let mut numbers = text.split('.').map(|part| {
            // Checked by hand because u64's parser accepts a leading "+".
            part.bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| part.parse::<u64>().ok())
                .flatten()
        });
        let version = Self {
            major: numbers.next()??,
            minor: numbers.next()??,
            patch: numbers.next()??,
        };
        numbers.next().is_none().then_some(version)
    }

    /// The version of this binary.
    pub fn installed() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is X.Y.Z")
    }

    /// The version a release's tag page belongs to: the last path segment of
    /// `…/releases/tag/vX.Y.Z`.
    fn from_tag_url(url: &str) -> Option<Self> {
        let (_, tag) = url.rsplit_once('/')?;
        Self::parse(tag.strip_prefix('v')?)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The place releases are published.
#[derive(Debug)]
pub struct Releases {
    base: String,
}

impl Releases {
    /// The releases `RECOLLECT_DOWNLOAD_BASE` names, else the project's on
    /// GitHub. A development build has no default, so that nothing it does
    /// (a test run above all) reaches GitHub.
    pub fn from_env() -> Result<Self> {
        let named = std::env::var("RECOLLECT_DOWNLOAD_BASE")
            .ok()
            .filter(|base| !base.is_empty());
        Self::at(named, cfg!(debug_assertions))
    }

    fn at(named: Option<String>, development_build: bool) -> Result<Self> {
        match named {
            Some(base) => Ok(Self {
                base: base.trim_end_matches('/').to_string(),
            }),
            None if development_build => Err(Error::DevelopmentBuildWithoutDownloadBase),
            None => Ok(Self {
                base: DEFAULT_DOWNLOAD_BASE.to_string(),
            }),
        }
    }

    /// The latest release: `<base>/latest` redirects to its tag page, and
    /// the redirect is read, not followed. On GitHub that is the newest
    /// release that is neither a draft nor a pre-release.
    pub fn latest(&self) -> Result<Version> {
        let url = format!("{}/latest", self.base);
        let no_answer = |reason: String| Error::ReleaseLookup {
            url: url.clone(),
            reason,
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(LOOKUP_TIMEOUT))
            .build()
            .into();
        let response = agent
            .get(&url)
            .call()
            .map_err(|err| no_answer(err.to_string()))?;
        response
            .headers()
            .get("location")
            .and_then(|location| location.to_str().ok())
            .and_then(Version::from_tag_url)
            .ok_or_else(|| {
                no_answer(format!(
                    "no release in the answer (HTTP {})",
                    response.status().as_u16()
                ))
            })
    }
}

/// What the last lookup of the latest release found, kept in `CHECK_FILE`
/// so that a machine asks once a day and not once a session.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    /// When a lookup was last attempted, whether it got an answer or not.
    checked_at: Option<String>,
    /// The latest release a lookup has returned; kept when an attempt fails.
    latest: Option<String>,
}

impl Check {
    /// The remembered check; an empty one when the file is missing or
    /// cannot be read.
    pub fn read(data_dir: &Path) -> Self {
        std::fs::read_to_string(data_dir.join(CHECK_FILE))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Records an attempt made at `now` and writes the file; an answer
    /// replaces the remembered release. A file that cannot be written is
    /// not worth failing for: the next session looks again.
    pub fn record(&mut self, data_dir: &Path, now: DateTime<Utc>, answer: Option<Version>) {
        self.checked_at = Some(format_timestamp(now));
        if let Some(latest) = answer {
            self.latest = Some(latest.to_string());
        }
        let _ = self.write(data_dir);
    }

    /// Writes under a temporary name first, so that a session starting at
    /// the same moment never reads half a file.
    fn write(&self, data_dir: &Path) -> std::io::Result<()> {
        let temporary = data_dir.join(format!("{CHECK_FILE}.{}", std::process::id()));
        std::fs::write(&temporary, serde_json::to_string(self)?)?;
        std::fs::rename(&temporary, data_dir.join(CHECK_FILE)).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    /// Whether it is time to look again: never looked, looked a day ago or
    /// longer, or the remembered time is not a time in the past (a clock
    /// that was set back must not silence the check until that time comes).
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        let checked_at = self
            .checked_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok());
        match checked_at {
            Some(at) => {
                let age = now.signed_duration_since(at);
                age < TimeDelta::zero() || age >= CHECK_INTERVAL
            }
            None => true,
        }
    }

    /// The remembered release, if it is newer than `installed`.
    pub fn newer_than(&self, installed: Version) -> Option<Version> {
        self.latest
            .as_deref()
            .and_then(Version::parse)
            .filter(|latest| *latest > installed)
    }
}

/// That release `latest` exists, for someone running `installed`.
pub fn available(latest: Version, installed: Version) -> String {
    format!("recollect {latest} is available (installed: {installed})")
}

/// What a starting session is told about a newer release. The reader is an
/// agent, and updating is the user's decision: an update can migrate the
/// database and stop sync with machines that are not updated yet.
pub fn notice(latest: Version, installed: Version) -> String {
    format!(
        "{}. Tell the user they can run `recollect update`; do not run it yourself.",
        available(latest, installed)
    )
}

/// The notice for a session starting at `now`, if a release newer than
/// `installed` is known. `lookup` asks for the latest release and is called
/// only when the remembered check is due; `None` from it is an attempt
/// without an answer.
pub fn session_notice(
    data_dir: &Path,
    installed: Version,
    now: DateTime<Utc>,
    lookup: impl FnOnce() -> Option<Version>,
) -> Option<String> {
    let mut check = Check::read(data_dir);
    if check.due(now) {
        check.record(data_dir, now, lookup());
    }
    check
        .newer_than(installed)
        .map(|latest| notice(latest, installed))
}

/// install.sh as it was when this binary was built. An old binary installs
/// newer releases with it, so the names and layout of the release files
/// must not change.
const INSTALL_SCRIPT: &str = include_str!("../install.sh");

/// The directory `recollect update` installs into: the one that holds the
/// running executable, symlinks resolved.
pub fn install_dir() -> Result<PathBuf> {
    let executable = std::env::current_exe()
        .and_then(|path| path.canonicalize())
        .map_err(|err| Error::Update(format!("cannot find the running recollect: {err}")))?;
    install_dir_of(&executable)
}

/// The directory of `executable`, refused when the installer could not
/// replace `executable` there: it installs a file named `recollect` and
/// must be able to write to the directory.
fn install_dir_of(executable: &Path) -> Result<PathBuf> {
    if executable.file_name() != Some(OsStr::new("recollect")) {
        return Err(Error::Update(format!(
            "{} is not named recollect, so the installer would not replace it; install with install.sh instead",
            executable.display()
        )));
    }
    let dir = executable
        .parent()
        .expect("a file is in a directory")
        .to_path_buf();
    // Tried by creating a file: permission bits do not tell about read-only
    // mounts and access lists.
    let probe = dir.join(format!(".recollect.writable.{}", std::process::id()));
    std::fs::File::create(&probe)
        .map_err(|err| Error::Update(format!("cannot write to {}: {err}", dir.display())))?;
    let _ = std::fs::remove_file(&probe);
    Ok(dir)
}

/// Installs release `version` into `install_dir` with the embedded
/// install.sh, whose output goes where this process's output goes. Returns
/// the script's exit status; a script that fails has said why and has
/// replaced nothing.
pub fn install(version: Version, install_dir: &Path) -> Result<ExitStatus> {
    let cannot_run =
        |err: std::io::Error| Error::Update(format!("cannot run the installer with sh: {err}"));
    let mut installer = Command::new("sh")
        .arg("-s")
        .env("RECOLLECT_VERSION", format!("v{version}"))
        .env("RECOLLECT_INSTALL_DIR", install_dir)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(cannot_run)?;
    let mut script = installer.stdin.take().expect("stdin is piped");
    // A script that ends early closes the pipe; its exit status tells.
    let _ = script.write_all(INSTALL_SCRIPT.as_bytes());
    drop(script);
    installer.wait().map_err(cannot_run)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    use chrono::TimeZone;

    use super::*;
    use crate::sync::test_support::closed_address;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 7, hour, minute, 0).unwrap()
    }

    /// Releases at a local server that answers its first request with `response`.
    fn releases_answering(response: &'static str) -> Releases {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut line = String::new();
            // The request ends at the first empty line.
            while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                line.clear();
            }
            socket.write_all(response.as_bytes()).unwrap();
        });
        Releases::at(Some(base), true).unwrap()
    }

    #[test]
    fn versions_are_three_numbers_and_compare_by_number() {
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("0.99.99"));
        assert_eq!(version("0.2.0"), version("0.2.0"));
        assert_eq!(version("0.3.1").to_string(), "0.3.1");
        for not_a_version in [
            "",
            "1.2",
            "1.2.3.4",
            "v1.2.3",
            "1.2.3-rc1",
            "+1.2.3",
            "1..3",
            "a.b.c",
        ] {
            assert_eq!(Version::parse(not_a_version), None, "{not_a_version:?}");
        }
    }

    #[test]
    fn the_installed_version_is_the_crate_version() {
        assert_eq!(Version::installed().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn only_a_version_tag_page_names_a_release() {
        for (url, expected) in [
            (
                "https://github.com/jkraemer/recollect/releases/tag/v0.3.0",
                Some("0.3.0"),
            ),
            ("/jkraemer/recollect/releases/tag/v1.2.3", Some("1.2.3")),
            (
                "https://github.com/jkraemer/recollect/releases/tag/nightly",
                None,
            ),
            (
                "https://github.com/jkraemer/recollect/releases/tag/v1.2",
                None,
            ),
            (
                "https://github.com/jkraemer/recollect/releases/tag/v1.2.3-rc1",
                None,
            ),
            (
                "https://github.com/jkraemer/recollect/releases/tag/v1.2.3/",
                None,
            ),
            (
                "https://github.com/jkraemer/recollect/releases/tag/v1.2.3?x=1",
                None,
            ),
            ("https://github.com/jkraemer/recollect/releases", None),
            ("v1.2.3", None),
            ("", None),
        ] {
            assert_eq!(Version::from_tag_url(url), expected.map(version), "{url:?}");
        }
    }

    #[test]
    fn releases_come_from_the_named_base_else_from_github() {
        let named = Releases::at(Some("http://localhost:1/releases/".into()), true).unwrap();
        assert_eq!(named.base, "http://localhost:1/releases");
        let default = Releases::at(None, false).unwrap();
        assert_eq!(
            default.base,
            "https://github.com/jkraemer/recollect/releases"
        );
    }

    #[test]
    fn a_development_build_looks_nothing_up_without_a_named_base() {
        let err = Releases::at(None, true).unwrap_err();
        assert_eq!(
            err.to_string(),
            "this is a development build; set RECOLLECT_DOWNLOAD_BASE to the releases it may look up and install"
        );
    }

    #[test]
    fn the_latest_release_is_the_one_the_redirect_names() {
        let releases = releases_answering(
            "HTTP/1.1 302 Found\r\nLocation: https://github.com/jkraemer/recollect/releases/tag/v0.3.0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(releases.latest().unwrap(), version("0.3.0"));
    }

    #[test]
    fn an_answer_that_names_no_release_is_an_error_with_its_status() {
        for (response, status) in [
            (
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                404,
            ),
            (
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                200,
            ),
            (
                "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                302,
            ),
        ] {
            let releases = releases_answering(response);
            let err = releases.latest().unwrap_err();
            assert_eq!(
                err.to_string(),
                format!(
                    "could not look up the latest release at {}/latest: no release in the answer (HTTP {status})",
                    releases.base
                )
            );
        }
    }

    #[test]
    fn a_site_that_cannot_be_reached_is_an_error() {
        let releases = Releases::at(Some(format!("http://{}", closed_address())), true).unwrap();
        let err = releases.latest().unwrap_err().to_string();
        assert!(
            err.starts_with(&format!(
                "could not look up the latest release at {}/latest: ",
                releases.base
            )),
            "{err}"
        );
    }

    #[test]
    fn a_site_that_never_answers_is_given_up_on() {
        // Connections are accepted by the system and never read.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let releases = Releases::at(Some(base), true).unwrap();
        let started = Instant::now();
        assert!(releases.latest().is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "gave up after {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_recorded_check_is_read_back() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Check::read(dir.path()), Check::default());
        let mut check = Check::default();
        check.record(dir.path(), at(9, 0), Some(version("0.3.0")));
        assert_eq!(Check::read(dir.path()), check);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(CHECK_FILE)).unwrap(),
            r#"{"checked_at":"2026-10-07T09:00:00.000Z","latest":"0.3.0"}"#
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [CHECK_FILE], "no temporary file left");
    }

    #[test]
    fn an_unreadable_check_file_counts_as_never_checked() {
        let dir = tempfile::tempdir().unwrap();
        for content in ["", "not json", "[]", r#"{"checked_at": 5}"#] {
            std::fs::write(dir.path().join(CHECK_FILE), content).unwrap();
            assert_eq!(Check::read(dir.path()), Check::default(), "{content:?}");
        }
        assert!(Check::default().due(at(9, 0)));
    }

    #[test]
    fn a_check_is_due_after_a_day_and_when_its_time_makes_no_sense() {
        let dir = tempfile::tempdir().unwrap();
        let mut check = Check::default();
        check.record(dir.path(), at(9, 0), None);
        assert!(!check.due(at(9, 0)));
        assert!(!check.due(at(9, 0) + TimeDelta::hours(24) - TimeDelta::seconds(1)));
        assert!(check.due(at(9, 0) + TimeDelta::hours(24)));
        // The clock was set back: the remembered time has not come yet.
        assert!(check.due(at(8, 59)));
        let garbled = Check {
            checked_at: Some("yesterday".into()),
            latest: None,
        };
        assert!(garbled.due(at(9, 0)));
    }

    #[test]
    fn a_failed_attempt_keeps_the_release_known_before() {
        let dir = tempfile::tempdir().unwrap();
        let mut check = Check::default();
        check.record(dir.path(), at(9, 0), Some(version("0.3.0")));
        check.record(dir.path(), at(10, 0), None);
        assert_eq!(check.newer_than(version("0.2.0")), Some(version("0.3.0")));
        assert!(!check.due(at(10, 1)), "the attempt counts as a check");
    }

    #[test]
    fn only_a_newer_release_is_worth_mentioning() {
        let dir = tempfile::tempdir().unwrap();
        let mut check = Check::default();
        assert_eq!(check.newer_than(version("0.2.0")), None);
        check.record(dir.path(), at(9, 0), Some(version("0.3.0")));
        assert_eq!(check.newer_than(version("0.2.0")), Some(version("0.3.0")));
        assert_eq!(check.newer_than(version("0.3.0")), None);
        assert_eq!(check.newer_than(version("0.4.0")), None);
    }

    #[test]
    fn the_notice_tells_the_agent_to_leave_the_update_to_the_user() {
        assert_eq!(
            notice(version("0.3.0"), version("0.2.0")),
            "recollect 0.3.0 is available (installed: 0.2.0). Tell the user they can run `recollect update`; do not run it yourself."
        );
        assert_eq!(
            available(version("0.3.0"), version("0.2.0")),
            "recollect 0.3.0 is available (installed: 0.2.0)"
        );
    }

    #[test]
    fn a_session_looks_up_the_latest_release_when_the_check_is_due() {
        let dir = tempfile::tempdir().unwrap();
        let said = session_notice(dir.path(), version("0.2.0"), at(9, 0), || {
            Some(version("0.3.0"))
        });
        assert_eq!(said, Some(notice(version("0.3.0"), version("0.2.0"))));
        assert!(!Check::read(dir.path()).due(at(9, 1)));
    }

    #[test]
    fn within_a_day_a_session_repeats_the_notice_without_looking_again() {
        let dir = tempfile::tempdir().unwrap();
        session_notice(dir.path(), version("0.2.0"), at(9, 0), || {
            Some(version("0.3.0"))
        });
        let said = session_notice(dir.path(), version("0.2.0"), at(17, 0), || {
            panic!("looked up again within a day")
        });
        assert_eq!(said, Some(notice(version("0.3.0"), version("0.2.0"))));
    }

    #[test]
    fn a_session_says_nothing_without_a_newer_release() {
        let dir = tempfile::tempdir().unwrap();
        for answer in [None, Some(version("0.2.0")), Some(version("0.1.0"))] {
            let now = at(9, 0) + TimeDelta::days(1);
            std::fs::remove_file(dir.path().join(CHECK_FILE)).ok();
            assert_eq!(
                session_notice(dir.path(), version("0.2.0"), now, || answer),
                None,
                "{answer:?}"
            );
            assert!(
                !Check::read(dir.path()).due(now),
                "the attempt is remembered: {answer:?}"
            );
        }
    }

    #[test]
    fn a_check_file_that_cannot_be_written_does_not_cost_the_notice() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-directory");
        let said = session_notice(&missing, version("0.2.0"), at(9, 0), || {
            Some(version("0.3.0"))
        });
        assert_eq!(said, Some(notice(version("0.3.0"), version("0.2.0"))));
    }

    #[test]
    fn the_install_directory_is_the_one_holding_recollect() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("recollect");
        std::fs::write(&binary, "").unwrap();
        assert_eq!(install_dir_of(&binary).unwrap(), dir.path());
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["recollect"], "the write test leaves nothing behind");
    }

    #[test]
    fn a_binary_with_another_name_has_no_install_directory() {
        let err = install_dir_of(Path::new("/opt/tools/recollect-dev")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "/opt/tools/recollect-dev is not named recollect, so the installer would not replace it; install with install.sh instead"
        );
    }

    #[test]
    fn a_directory_that_cannot_be_written_to_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone").join("recollect");
        let err = install_dir_of(&missing).unwrap_err().to_string();
        assert!(
            err.starts_with(&format!(
                "cannot write to {}: ",
                dir.path().join("gone").display()
            )),
            "{err}"
        );
    }

    #[test]
    fn the_installer_is_the_install_script_of_this_checkout() {
        assert!(INSTALL_SCRIPT.starts_with("#!/bin/sh\n"));
        assert!(INSTALL_SCRIPT.contains("RECOLLECT_INSTALL_DIR"));
    }
}
