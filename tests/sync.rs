//! Sync end to end: the real binary on machines with a data directory each,
//! talking over local sockets.

mod common;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::*;
use recollect::db::Database;
use serde_json::Value;
use tempfile::TempDir;

/// One machine: a data directory with its own name, key and database.
struct Machine {
    dir: TempDir,
    name: String,
}

impl Machine {
    /// A machine called `name` whose daemon listens on a free local port and
    /// syncs every `interval_seconds`.
    fn new(name: &str, interval_seconds: u64) -> Self {
        let machine = Self {
            dir: tempfile::tempdir().unwrap(),
            name: name.to_string(),
        };
        machine.configure("127.0.0.1:0", interval_seconds);
        machine
    }

    fn configure(&self, listen: &str, interval_seconds: u64) {
        std::fs::write(
            self.dir.path().join("config.toml"),
            format!(
                "[sync]\nlisten = \"{listen}\"\ninterval_seconds = {interval_seconds}\nname = \"{}\"\n",
                self.name
            ),
        )
        .unwrap();
    }

    /// `recollect` on this machine, with the model already downloaded so no
    /// download notice appears on stderr.
    fn recollect(&self) -> Command {
        let _ = common::shared_model();
        let mut command = Command::cargo_bin("recollect").unwrap();
        command
            .env("RECOLLECT_DATA_DIR", self.dir.path())
            .env("RECOLLECT_MODEL_DIR", common::model_dir());
        command
    }

    /// Runs a command that must succeed silently on stderr; parses its stdout as JSON.
    fn json(&self, args: &[&str]) -> Value {
        let output = self.recollect().args(args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn fingerprint(&self) -> String {
        self.json(&["id", "--json"])["fingerprint"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn store(&self, content: &str) {
        self.recollect()
            .args(["store", content])
            .assert()
            .success()
            .stderr("");
    }

    /// The contents of the machine's memories, sorted.
    fn contents(&self) -> Vec<String> {
        let mut contents: Vec<String> = self
            .json(&["list", "--json", "--limit", "1000"])
            .as_array()
            .unwrap()
            .iter()
            .map(|memory| memory["content"].as_str().unwrap().to_string())
            .collect();
        contents.sort();
        contents
    }

    /// The local id of the memory with this content.
    fn id_of(&self, content: &str) -> String {
        self.json(&["list", "--json", "--limit", "1000"])
            .as_array()
            .unwrap()
            .iter()
            .find(|memory| memory["content"] == content)
            .unwrap_or_else(|| panic!("{} has no memory {content:?}", self.name))["id"]
            .to_string()
    }

    fn database(&self) -> Database {
        Database::open(&self.dir.path().join("memories.db")).unwrap()
    }

    /// Makes `other`, reached at `address`, a peer of this machine without
    /// a pairing.
    fn knows(&self, other: &Machine, address: &str) {
        self.database()
            .add_peer(
                &other.name,
                &other.fingerprint(),
                address,
                "2026-10-07T10:00:00.000Z",
            )
            .unwrap();
    }

    fn peer(&self, name: &str) -> Value {
        self.json(&["peer", "list", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .find(|peer| peer["name"] == name)
            .unwrap_or_else(|| panic!("{} has no peer {name}", self.name))
            .clone()
    }

    /// Starts `recollect serve` on this machine and waits until it listens.
    fn serve(&self) -> Daemon {
        let _ = common::shared_model();
        let mut child = StdCommand::new(assert_cmd::cargo::cargo_bin("recollect"))
            .arg("serve")
            .env("RECOLLECT_DATA_DIR", self.dir.path())
            .env("RECOLLECT_MODEL_DIR", common::model_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let first = lines.next().expect("the daemon wrote nothing").unwrap();
        let address = first
            .strip_prefix("listening on ")
            .unwrap_or_else(|| panic!("unexpected first line: {first}"))
            .to_string();
        let log = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in lines.map_while(Result::ok) {
                collected.lock().unwrap().push(line);
            }
        });
        Daemon {
            child,
            address,
            log,
        }
    }
}

/// A running `recollect serve`; stopped when dropped.
struct Daemon {
    child: Child,
    /// Where it listens, `127.0.0.1:<port>`.
    address: String,
    log: Arc<Mutex<Vec<String>>>,
}

impl Daemon {
    /// What it has logged since it started listening.
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Polls `check` for up to twenty seconds; panics with `what` if it never holds.
fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A local address nothing listens on.
fn closed_address() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

#[test]
fn serve_announces_where_it_listens() {
    let machine = Machine::new("alpha", 3600);
    let daemon = machine.serve();
    let port = daemon.address.strip_prefix("127.0.0.1:").unwrap();
    assert_ne!(port.parse::<u16>().unwrap(), 0);
}

#[test]
fn a_second_daemon_on_the_same_port_says_the_port_is_taken() {
    let first = Machine::new("alpha", 3600);
    let daemon = first.serve();
    let second = Machine::new("beta", 3600);
    second.configure(&daemon.address, 3600);
    second
        .recollect()
        .arg("serve")
        .timeout(Duration::from_secs(20))
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::starts_with(format!(
            "error: cannot listen on {}: ",
            daemon.address
        )));
}

#[test]
fn id_shows_the_name_the_fingerprint_and_the_listen_address() {
    let machine = Machine::new("alpha", 3600);
    let fingerprint = machine.fingerprint();
    assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
    assert_eq!(
        machine.json(&["id", "--json"]),
        serde_json::json!({
            "name": "alpha", "fingerprint": fingerprint, "listen": "127.0.0.1:0"
        })
    );
    machine
        .recollect()
        .arg("id")
        .assert()
        .success()
        .stdout(format!(
            "name:        alpha\nfingerprint: {fingerprint}\nlisten:      127.0.0.1:0\n"
        ))
        .stderr("");
}

#[test]
fn peers_are_listed_readdressed_and_removed() {
    let (alpha, beta) = (Machine::new("alpha", 3600), Machine::new("beta", 3600));
    alpha
        .recollect()
        .args(["peer", "list"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
    alpha.knows(&beta, "beta.example:7327");
    let fingerprint = beta.fingerprint();
    alpha
        .recollect()
        .args(["peer", "list"])
        .assert()
        .success()
        .stdout(format!(
            "beta · beta.example:7327 · {fingerprint} · never synced\n"
        ))
        .stderr("");

    alpha
        .recollect()
        .args(["peer", "address", "beta", "10.0.0.2:7327"])
        .assert()
        .success()
        .stdout("beta is now dialled at 10.0.0.2:7327\n")
        .stderr("");
    assert_eq!(alpha.peer("beta")["address"], "10.0.0.2:7327");
    alpha
        .recollect()
        .args(["peer", "address", "beta", "nowhere"])
        .assert()
        .failure()
        .stderr("error: invalid address \"nowhere\": use host:port, such as foehn:7327\n");
    alpha
        .recollect()
        .args(["peer", "address", "gamma", "10.0.0.3:7327"])
        .assert()
        .failure()
        .stderr("error: no peer named \"gamma\"\n");

    alpha
        .recollect()
        .args(["peer", "remove", "beta"])
        .assert()
        .success()
        .stdout("removed beta\n")
        .stderr("");
    assert_eq!(
        alpha.json(&["peer", "list", "--json"]),
        serde_json::json!([])
    );
    alpha
        .recollect()
        .args(["peer", "remove", "beta"])
        .assert()
        .failure()
        .stderr("error: no peer named \"beta\"\n");
}

#[test]
fn two_daemons_sync_on_their_timers_and_embed_what_they_receive() {
    let (alpha, beta) = (Machine::new("alpha", 1), Machine::new("beta", 1));
    let (alpha_daemon, beta_daemon) = (alpha.serve(), beta.serve());
    alpha.knows(&beta, &beta_daemon.address);
    beta.knows(&alpha, &alpha_daemon.address);
    let about_storage = "We keep every memory in a single SQLite file with a project column.";
    alpha.store(about_storage);
    beta.store("Banana bread needs very ripe bananas.");

    let both = ["Banana bread needs very ripe bananas.", about_storage];
    eventually("both machines hold both memories", || {
        alpha.contents() == both && beta.contents() == both
    });
    eventually("beta has embedded what it received", || {
        beta.json(&["status", "--json"])["pending_embeddings"] == 0
    });
    // No word of the query occurs in the memory: only vectors made on beta find it.
    let found = beta.json(&["search", "how is data persisted on disk", "--json"]);
    assert_eq!(found[0]["content"], about_storage);

    alpha
        .recollect()
        .args(["delete", &alpha.id_of(about_storage)])
        .assert()
        .success();
    eventually("the deletion reached beta", || {
        beta.contents() == ["Banana bread needs very ripe bananas."]
    });

    let logged = [alpha_daemon.log(), beta_daemon.log()].concat();
    assert!(
        logged.iter().any(|line| line.contains("1 memory")),
        "a round that moved something is logged: {logged:?}"
    );
    assert!(
        logged.iter().all(|line| !line.contains("error")),
        "{logged:?}"
    );
    assert!(alpha.peer("beta")["last_sync_at"].is_string());
}

#[test]
fn an_unreachable_peer_is_logged_once_and_shown_by_peer_list() {
    let (alpha, beta) = (Machine::new("alpha", 1), Machine::new("beta", 3600));
    let nowhere = closed_address();
    alpha.knows(&beta, &nowhere);
    let daemon = alpha.serve();

    eventually("the failure is recorded", || {
        alpha.peer("beta")["last_error"].is_string()
    });
    // Time for at least two more rounds.
    std::thread::sleep(Duration::from_millis(2500));

    let failures: Vec<String> = daemon
        .log()
        .into_iter()
        .filter(|line| line.starts_with("beta: error: "))
        .collect();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].starts_with(&format!("beta: error: {nowhere}: cannot connect (")),
        "{failures:?}"
    );
    let peer = alpha.peer("beta");
    assert_eq!(peer["last_sync_at"], Value::Null);
    alpha
        .recollect()
        .args(["peer", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "never synced\n  error: {nowhere}: cannot connect ("
        )));
}
