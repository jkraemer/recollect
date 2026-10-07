//! A fake release site: an HTTP server on a local port that answers like
//! the project's releases on GitHub.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Stands in for `https://github.com/jkraemer/recollect/releases`.
pub struct ReleaseServer {
    /// What `RECOLLECT_DOWNLOAD_BASE` is set to.
    pub base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl ReleaseServer {
    /// Serves the files under `root`. `/latest` redirects to the tag page
    /// of `latest` (a tag such as `v9.9.9`); without one it is not found.
    pub fn start(root: &Path, latest: Option<&str>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let site = Site {
            root: root.to_path_buf(),
            tag_page: latest.map(|tag| format!("{base}/tag/{tag}")),
            requests: Arc::clone(&requests),
        };
        // Ends with the test process.
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                site.answer(socket);
            }
        });
        Self { base, requests }
    }

    /// The paths requested so far, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

struct Site {
    root: PathBuf,
    tag_page: Option<String>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Site {
    /// Answers one request and closes the connection.
    fn answer(&self, mut socket: TcpStream) {
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        // The headers end at the first empty line.
        let mut header = String::new();
        while reader.read_line(&mut header).is_ok_and(|read| read > 2) {
            header.clear();
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        self.requests.lock().unwrap().push(path.clone());
        let _ = socket.write_all(&self.response(&path));
    }

    fn response(&self, path: &str) -> Vec<u8> {
        const CLOSE: &str = "Connection: close\r\n\r\n";
        if let ("/latest", Some(tag_page)) = (path, &self.tag_page) {
            return format!(
                "HTTP/1.1 302 Found\r\nLocation: {tag_page}\r\nContent-Length: 0\r\n{CLOSE}"
            )
            .into_bytes();
        }
        match std::fs::read(self.root.join(path.trim_start_matches('/'))) {
            Ok(body) => {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{CLOSE}",
                    body.len()
                )
                .into_bytes();
                response.extend(body);
                response
            }
            Err(_) => {
                format!("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n{CLOSE}").into_bytes()
            }
        }
    }
}

/// Every target install.sh can pick, so a fake release serves any of them.
pub const TARGETS: [&str; 3] = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
];

/// Publishes a release into `dir` (`…/latest/download` or
/// `…/download/<tag>`): for every target an archive whose `recollect` is a
/// shell stub running `body_for(target)`, and SHA256SUMS.
pub fn publish(dir: &Path, body_for: impl Fn(&str) -> String) {
    std::fs::create_dir_all(dir).unwrap();
    let staging = tempfile::tempdir().unwrap();
    let mut sums = String::new();
    for target in TARGETS {
        write_script(&staging.path().join("recollect"), &body_for(target));
        let archive = format!("recollect-{target}.tar.gz");
        let status = Command::new("tar")
            .arg("-czf")
            .arg(dir.join(&archive))
            .arg("-C")
            .arg(staging.path())
            .arg("recollect")
            .status()
            .unwrap();
        assert!(status.success(), "tar failed");
        sums.push_str(&format!("{}  {archive}\n", sha256(&dir.join(&archive))));
    }
    std::fs::write(dir.join("SHA256SUMS"), sums).unwrap();
}

/// An executable shell script at `path`.
pub fn write_script(path: &Path, body: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// The SHA-256 of `file` in hex, from sha256sum or shasum, whichever exists.
pub fn sha256(file: &Path) -> String {
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
