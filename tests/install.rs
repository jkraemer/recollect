//! install.sh, run against a fake release served from a temporary directory
//! through file:// URLs, with a stub `recollect` in place of the real binary.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

/// Every target install.sh can pick, so the fake release serves any of them.
const TARGETS: [&str; 3] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
];

/// Where curl, tar, sha256sum or shasum, uname and the other tools the
/// script uses live on Linux and macOS.
const SYSTEM_PATH: &str = "/usr/bin:/bin";

const MODEL_NOTE: &str = "The embedding model (about 30 MB) downloads on first use.";

struct Fixture {
    dir: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn releases(&self) -> PathBuf {
        self.dir.path().join("releases")
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn install_dir(&self) -> PathBuf {
        self.home().join(".local/bin")
    }

    fn installed(&self) -> PathBuf {
        self.install_dir().join("recollect")
    }

    /// PATH with the default install directory, as on a set-up machine.
    fn path_with_install_dir(&self) -> String {
        format!("{}:{SYSTEM_PATH}", self.install_dir().display())
    }

    /// A directory under the fixture, created.
    fn subdir(&self, name: &str) -> PathBuf {
        let dir = self.dir.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Publishes a release at `releases/<location>` (`latest/download` or
    /// `download/<tag>`) whose `recollect` is a shell stub running `body`.
    fn publish(&self, location: &str, body: &str) {
        self.publish_per_target(location, |_| body.to_string());
    }

    /// Like `publish`, with the stub in each target's archive running
    /// `body_for(target)`.
    fn publish_per_target(&self, location: &str, body_for: impl Fn(&str) -> String) {
        let dir = self.releases().join(location);
        fs::create_dir_all(&dir).unwrap();
        let staging = self.subdir("staging");
        let mut sums = String::new();
        for target in TARGETS {
            write_script(&staging.join("recollect"), &body_for(target));
            let archive = format!("recollect-{target}.tar.gz");
            let status = Command::new("tar")
                .arg("-czf")
                .arg(dir.join(&archive))
                .arg("-C")
                .arg(&staging)
                .arg("recollect")
                .status()
                .unwrap();
            assert!(status.success(), "tar failed");
            sums.push_str(&format!("{}  {archive}\n", sha256(&dir.join(&archive))));
        }
        fs::remove_dir_all(&staging).unwrap();
        fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    }

    /// Publishes a release whose stub prints `recollect <version>`.
    fn publish_version(&self, location: &str, version: &str) {
        self.publish(location, &format!("echo \"recollect {version}\""));
    }

    /// An installed `recollect` that prints `recollect 0.0.1`.
    fn install_old_version(&self) {
        fs::create_dir_all(self.install_dir()).unwrap();
        write_script(&self.installed(), "echo \"recollect 0.0.1\"");
    }

    /// Rewrites the latest release's SHA256SUMS: `rewrite(archive, hash)`
    /// returns the line's new hash, or None to drop the line.
    fn rewrite_sums(&self, rewrite: impl Fn(&str, &str) -> Option<String>) {
        let sums = self.releases().join("latest/download/SHA256SUMS");
        let rewritten: String = fs::read_to_string(&sums)
            .unwrap()
            .lines()
            .filter_map(|line| {
                let (hash, archive) = line.split_once("  ").unwrap();
                rewrite(archive, hash).map(|hash| format!("{hash}  {archive}\n"))
            })
            .collect();
        fs::write(&sums, rewritten).unwrap();
    }

    /// PATH with the default install directory behind a `uname` stub that
    /// reports `os` and `arch`.
    fn path_on_platform(&self, os: &str, arch: &str) -> String {
        let stubs = self.subdir("stubs");
        write_script(
            &stubs.join("uname"),
            &format!("case \"$1\" in -s) echo {os} ;; -m) echo {arch} ;; esac"),
        );
        format!("{}:{}", stubs.display(), self.path_with_install_dir())
    }

    /// `/bin/sh` with only the variables a fresh shell would have, the fake
    /// releases as download base, and `path` as PATH.
    fn command(&self, path: &str, env: &[(&str, &str)]) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", path)
            .env(
                "RECOLLECT_DOWNLOAD_BASE",
                format!("file://{}", self.releases().display()),
            );
        for (key, value) in env {
            command.env(key, value);
        }
        command
    }

    /// `sh install.sh` against the fake releases.
    fn run(&self, path: &str, env: &[(&str, &str)]) -> Output {
        self.command(path, env).arg(script()).output().unwrap()
    }
}

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("install.sh")
}

fn write_script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The SHA-256 of `file` in hex, from sha256sum or shasum, whichever exists.
fn sha256(file: &Path) -> String {
    for (program, args) in [("sha256sum", &[][..]), ("shasum", &["-a", "256"][..])] {
        if let Ok(output) = Command::new(program).args(args).arg(file).output()
            && output.status.success()
        {
            let line = String::from_utf8(output.stdout).unwrap();
            return line.split_whitespace().next().unwrap().to_string();
        }
    }
    panic!("neither sha256sum nor shasum is available");
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The names in `dir`.
fn entries(dir: &Path) -> Vec<OsString> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect()
}

fn version_of(binary: &Path) -> String {
    text(&Command::new(binary).output().unwrap().stdout)
}

#[test]
fn installs_the_latest_release_into_a_new_local_bin() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let output = fixture.run(&fixture.path_with_install_dir(), &[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        text(&output.stdout),
        format!(
            "installed recollect 9.9.9 to {}\n{MODEL_NOTE}\n",
            fixture.installed().display()
        )
    );
    assert_eq!(version_of(&fixture.installed()), "recollect 9.9.9\n");
    let mode = fs::metadata(fixture.installed())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o755, 0o755);
}

#[test]
fn an_upgrade_replaces_the_installed_binary_in_one_step() {
    let fixture = Fixture::new();
    fixture.install_old_version();
    fixture.publish_version("latest/download", "9.9.9");
    let output = fixture.run(&fixture.path_with_install_dir(), &[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(version_of(&fixture.installed()), "recollect 9.9.9\n");
    assert_eq!(
        entries(&fixture.install_dir()),
        [OsString::from("recollect")],
        "no temporary file left"
    );
}

#[test]
fn a_checksum_mismatch_stops_before_anything_is_replaced() {
    let fixture = Fixture::new();
    fixture.install_old_version();
    fixture.publish_version("latest/download", "9.9.9");
    fixture.rewrite_sums(|_, _| Some("0".repeat(64)));
    let output = fixture.run(&fixture.path_with_install_dir(), &[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.starts_with("error: checksum mismatch for recollect-"),
        "{stderr}"
    );
    assert_eq!(version_of(&fixture.installed()), "recollect 0.0.1\n");
}

#[test]
fn a_pinned_version_comes_from_that_release() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    fixture.publish_version("download/v1.2.3", "1.2.3");
    let output = fixture.run(
        &fixture.path_with_install_dir(),
        &[("RECOLLECT_VERSION", "v1.2.3")],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(version_of(&fixture.installed()), "recollect 1.2.3\n");
}

#[test]
fn the_install_directory_can_be_chosen() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let elsewhere = fixture.dir.path().join("elsewhere");
    let output = fixture.run(
        &format!("{}:{SYSTEM_PATH}", elsewhere.display()),
        &[("RECOLLECT_INSTALL_DIR", elsewhere.to_str().unwrap())],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        version_of(&elsewhere.join("recollect")),
        "recollect 9.9.9\n"
    );
    assert!(!fixture.installed().exists());
}

#[test]
fn each_platform_gets_the_archive_built_for_it() {
    for (os, arch, target) in [
        ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
        ("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
        ("Linux", "arm64", "aarch64-unknown-linux-gnu"),
        ("Darwin", "arm64", "aarch64-apple-darwin"),
    ] {
        let fixture = Fixture::new();
        fixture.publish_per_target("latest/download", |built_for| {
            format!("echo \"recollect for {built_for}\"")
        });
        let output = fixture.run(&fixture.path_on_platform(os, arch), &[]);
        assert!(
            output.status.success(),
            "{os} {arch}: {}",
            text(&output.stderr)
        );
        assert_eq!(
            version_of(&fixture.installed()),
            format!("recollect for {target}\n"),
            "{os} {arch}"
        );
    }
}

#[test]
fn a_release_without_a_checksum_for_this_platform_keeps_the_old_binary() {
    let fixture = Fixture::new();
    fixture.install_old_version();
    fixture.publish_version("latest/download", "9.9.9");
    fixture.rewrite_sums(|archive, hash| {
        (archive != "recollect-x86_64-unknown-linux-gnu.tar.gz").then(|| hash.to_string())
    });
    let output = fixture.run(&fixture.path_on_platform("Linux", "x86_64"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "error: SHA256SUMS lists no recollect-x86_64-unknown-linux-gnu.tar.gz\n"
    );
    assert_eq!(version_of(&fixture.installed()), "recollect 0.0.1\n");
}

#[test]
fn the_temporary_download_directory_is_removed_after_an_install() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let tmp = fixture.subdir("tmp");
    let output = fixture.run(
        &fixture.path_with_install_dir(),
        &[("TMPDIR", tmp.to_str().unwrap())],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(entries(&tmp), Vec::<OsString>::new());
}

#[test]
fn the_temporary_download_directory_is_removed_after_a_failed_install() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    fixture.rewrite_sums(|_, _| Some("0".repeat(64)));
    let tmp = fixture.subdir("tmp");
    let output = fixture.run(
        &fixture.path_with_install_dir(),
        &[("TMPDIR", tmp.to_str().unwrap())],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(entries(&tmp), Vec::<OsString>::new());
}

#[test]
fn a_trailing_slash_on_the_install_directory_changes_nothing() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let elsewhere = fixture.dir.path().join("elsewhere");
    let output = fixture.run(
        &format!("{}:{SYSTEM_PATH}", elsewhere.display()),
        &[(
            "RECOLLECT_INSTALL_DIR",
            &format!("{}/", elsewhere.display()),
        )],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stderr), "");
    assert_eq!(
        text(&output.stdout),
        format!(
            "installed recollect 9.9.9 to {}/recollect\n{MODEL_NOTE}\n",
            elsewhere.display()
        )
    );
    assert_eq!(
        version_of(&elsewhere.join("recollect")),
        "recollect 9.9.9\n"
    );
}

#[test]
fn a_missing_release_is_reported_and_installs_nothing() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let output = fixture.run(
        &fixture.path_with_install_dir(),
        &[("RECOLLECT_VERSION", "v0.0.0")],
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    let expected = format!(
        "error: could not download file://{}/download/v0.0.0/recollect-",
        fixture.releases().display()
    );
    assert!(stderr.contains(&expected), "{stderr}");
    assert!(!fixture.installed().exists());
}

#[test]
fn a_binary_that_cannot_run_here_keeps_the_old_one() {
    let fixture = Fixture::new();
    fixture.install_old_version();
    fixture.publish(
        "latest/download",
        "echo \"version 'GLIBC_2.35' not found\" >&2; exit 1",
    );
    let output = fixture.run(&fixture.path_with_install_dir(), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "error: the downloaded recollect does not run on this machine: version 'GLIBC_2.35' not found\n"
    );
    assert_eq!(version_of(&fixture.installed()), "recollect 0.0.1\n");
}

#[test]
fn an_unsupported_platform_is_refused() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let output = fixture.run(&fixture.path_on_platform("Darwin", "x86_64"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "error: no prebuilt recollect for Darwin x86_64\n"
    );
    assert!(!fixture.installed().exists());
}

#[test]
fn a_missing_curl_is_named() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    // Only uname, which the script needs before it looks for curl.
    let tools = fixture.subdir("tools");
    let uname = Command::new("/bin/sh")
        .args(["-c", "command -v uname"])
        .output()
        .unwrap();
    std::os::unix::fs::symlink(text(&uname.stdout).trim(), tools.join("uname")).unwrap();
    let output = fixture.run(tools.to_str().unwrap(), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        "error: curl is required to download recollect\n"
    );
}

#[test]
fn warns_when_the_install_directory_is_not_on_path() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let output = fixture.run(SYSTEM_PATH, &[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stderr),
        format!(
            "warning: {} is not on PATH\n",
            fixture.install_dir().display()
        )
    );
}

#[test]
fn warns_when_another_recollect_comes_first_on_path() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let gem_bin = fixture.subdir("gem-bin");
    write_script(&gem_bin.join("recollect"), "echo \"ruby recollect\"");
    let output = fixture.run(
        &format!("{}:{}", gem_bin.display(), fixture.path_with_install_dir()),
        &[],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stderr),
        format!(
            "warning: {}/recollect comes first on PATH and shadows {}\n",
            gem_bin.display(),
            fixture.installed().display()
        )
    );
}

#[test]
fn works_when_piped_into_sh() {
    let fixture = Fixture::new();
    fixture.publish_version("latest/download", "9.9.9");
    let mut child = fixture
        .command(&fixture.path_with_install_dir(), &[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&fs::read(script()).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(version_of(&fixture.installed()), "recollect 9.9.9\n");
}
