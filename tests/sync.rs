//! Sync end to end: the real binary on machines with a data directory each,
//! talking over local sockets.

mod common;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::*;
use recollect::db::Database;
use recollect::memory::{MemoryType, SyncRecord};
use recollect::sync::identity::Identity;
use recollect::sync::protocol::{EntryState, MESSAGE_LIMIT, Message, PROTOCOL_VERSION};
use recollect::sync::transport::{self, Timeouts};
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
        let stderr = child.stderr.take().unwrap();
        // The guard comes first: whatever fails below, the daemon is stopped.
        let mut daemon = Daemon {
            child,
            address: String::new(),
            log: Arc::default(),
        };
        let (first_line, first) = mpsc::channel();
        let collected = Arc::clone(&daemon.log);
        std::thread::spawn(move || {
            let mut lines = BufReader::new(stderr).lines().map_while(Result::ok);
            if let Some(line) = lines.next() {
                let _ = first_line.send(line);
            }
            for line in lines {
                collected.lock().unwrap().push(line);
            }
        });
        let first = match first.recv_timeout(Duration::from_secs(20)) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                panic!("{}'s daemon wrote nothing for 20 seconds", self.name)
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("{}'s daemon ended without a word", self.name)
            }
        };
        daemon.address = first
            .strip_prefix("listening on ")
            .unwrap_or_else(|| panic!("unexpected first line: {first}"))
            .to_string();
        daemon
    }
}

const PAIR_HINT: &str =
    "Run this on the other machine within 10 minutes; recollect serve must be running here.";

const NOT_PAIRED: &str =
    "this machine does not know your key; pair again with recollect pair and recollect join";

const OTHER_PROTOCOL: &str =
    "peer gamma speaks sync protocol 2, this recollect speaks 1; upgrade the older one";

impl Machine {
    /// Makes an invite on this machine, which other machines reach at `address`.
    fn pair(&self, address: &str) -> String {
        let output = self
            .recollect()
            .args(["pair", "--address", address])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(stderr.is_empty(), "{stderr}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let mut lines = stdout.lines();
        let invite = lines
            .next()
            .and_then(|line| line.strip_prefix("recollect join "))
            .unwrap_or_else(|| panic!("the join command comes first: {stdout}"))
            .to_string();
        assert_eq!(lines.next(), Some(PAIR_HINT));
        assert_eq!(lines.next(), None);
        invite
    }

    /// Pairs this machine, reached at `own_address`, with `inviter`, whose
    /// daemon listens at `inviter_address`.
    fn join(&self, inviter: &Machine, inviter_address: &str, own_address: &str) {
        let invite = inviter.pair(inviter_address);
        self.recollect()
            .args(["join", &invite, "--address", own_address])
            .assert()
            .success()
            .stdout(format!("paired with {0}\n{0}: in sync\n", inviter.name))
            .stderr("");
    }

    /// Runs `recollect sync` and expects exactly this on stdout.
    fn sync(&self, expected: &str) {
        self.recollect()
            .arg("sync")
            .assert()
            .success()
            .stdout(expected.to_string())
            .stderr("");
    }
}

/// Two machines with running daemons, paired by one join on `beta`.
fn paired() -> (Machine, Daemon, Machine, Daemon) {
    let (alpha, beta) = (Machine::new("alpha", 3600), Machine::new("beta", 3600));
    let (alpha_daemon, beta_daemon) = (alpha.serve(), beta.serve());
    beta.join(&alpha, &alpha_daemon.address, &beta_daemon.address);
    (alpha, alpha_daemon, beta, beta_daemon)
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
    // A round that fails while the two are still being introduced to each
    // other is made up for by the next one.
    eventually("neither machine has a failed round on record", || {
        alpha.peer("beta")["last_error"].is_null() && beta.peer("alpha")["last_error"].is_null()
    });
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

#[test]
fn a_corrected_address_is_dialled_and_the_daemon_says_it_syncs_again() {
    let (alpha, beta) = (Machine::new("alpha", 1), Machine::new("beta", 3600));
    // beta answers alpha's rounds and starts none: it learns of alpha after
    // the one pass it makes in this test.
    let beta_daemon = beta.serve();
    beta.knows(&alpha, &closed_address());
    alpha.knows(&beta, &closed_address());
    let alpha_daemon = alpha.serve();
    eventually("the failed dial is recorded", || {
        alpha.peer("beta")["last_error"].is_string()
    });

    alpha
        .recollect()
        .args(["peer", "address", "beta", &beta_daemon.address])
        .assert()
        .success()
        .stdout(format!("beta is now dialled at {}\n", beta_daemon.address))
        .stderr("");

    eventually("a round at the corrected address succeeded", || {
        let peer = alpha.peer("beta");
        peer["last_error"].is_null() && peer["last_sync_at"].is_string()
    });
    eventually("the daemon logged that it syncs again", || {
        alpha_daemon
            .log()
            .contains(&"beta: syncing again".to_string())
    });
    let about_beta: Vec<String> = alpha_daemon
        .log()
        .into_iter()
        .filter(|line| line.starts_with("beta: "))
        .collect();
    // Why the first dial failed is not checked: the port the test closed
    // may have gone to another test's listener in the meantime.
    assert_eq!(about_beta.len(), 2, "{about_beta:?}");
    assert!(about_beta[0].starts_with("beta: error: "), "{about_beta:?}");
    assert_eq!(about_beta[1], "beta: syncing again");
}

#[test]
fn one_join_gives_both_machines_the_same_knowledge_of_each_other() {
    let (alpha, beta) = (Machine::new("alpha", 3600), Machine::new("beta", 3600));
    let (alpha_daemon, beta_daemon) = (alpha.serve(), beta.serve());
    let invite = alpha.pair(&alpha_daemon.address);

    // Pasted with stray whitespace, as a copy from a terminal may be.
    beta.recollect()
        .args([
            "join",
            &format!("  {invite}\n"),
            "--address",
            &beta_daemon.address,
        ])
        .assert()
        .success()
        .stdout("paired with alpha\nalpha: in sync\n")
        .stderr("");

    let alpha_on_beta = beta.peer("alpha");
    assert_eq!(alpha_on_beta["fingerprint"], alpha.fingerprint());
    assert_eq!(alpha_on_beta["address"], alpha_daemon.address);
    assert!(
        alpha_on_beta["last_sync_at"].is_string(),
        "the join ran a first round"
    );
    let beta_on_alpha = alpha.peer("beta");
    assert_eq!(beta_on_alpha["fingerprint"], beta.fingerprint());
    assert_eq!(beta_on_alpha["address"], beta_daemon.address);
    eventually("alpha's daemon logged the pairing", || {
        alpha_daemon.log().contains(&"paired with beta".to_string())
    });
}

#[test]
fn sync_moves_memories_and_deletions_both_ways_whichever_machine_starts() {
    let (alpha, _alpha_daemon, beta, _beta_daemon) = paired();
    let about_storage = "We keep every memory in a single SQLite file with a project column.";
    alpha.store(about_storage);
    beta.store("from beta");

    // The joiner dials the inviter.
    beta.sync("alpha: received 1 memory, sent 1 memory\n");
    let both = [
        "We keep every memory in a single SQLite file with a project column.",
        "from beta",
    ];
    assert_eq!(alpha.contents(), both);
    assert_eq!(beta.contents(), both);
    assert_eq!(beta.json(&["status", "--json"])["pending_embeddings"], 0);
    let found = beta.json(&["search", "how is data persisted on disk", "--json"]);
    assert_eq!(
        found[0]["content"], about_storage,
        "found by vectors made on beta"
    );

    // The inviter dials the joiner: the pairing is symmetric.
    alpha
        .recollect()
        .args(["delete", &alpha.id_of(about_storage)])
        .assert()
        .success();
    beta.store("more from beta");
    alpha.sync("beta: received 1 memory, sent 1 deletion\n");
    let remaining = ["from beta", "more from beta"];
    assert_eq!(alpha.contents(), remaining);
    assert_eq!(beta.contents(), remaining);

    alpha.sync("beta: in sync\n");
    alpha
        .recollect()
        .args(["sync", "beta"])
        .assert()
        .success()
        .stdout("beta: in sync\n")
        .stderr("");
}

#[test]
fn an_invite_works_once_and_a_refused_join_pairs_nothing() {
    let (alpha, beta, gamma) = (
        Machine::new("alpha", 3600),
        Machine::new("beta", 3600),
        Machine::new("gamma", 3600),
    );
    let daemon = alpha.serve();
    let invite = alpha.pair(&daemon.address);
    let nowhere = closed_address();
    beta.recollect()
        .args(["join", &invite, "--address", &nowhere])
        .assert()
        .success();

    gamma
        .recollect()
        .args(["join", &invite, "--address", &nowhere])
        .assert()
        .failure()
        .stdout("")
        .stderr("error: alpha answered: this invite is not valid (expired or already used)\n");

    assert_eq!(
        gamma.json(&["peer", "list", "--json"]),
        serde_json::json!([])
    );
    let on_alpha = alpha.json(&["peer", "list", "--json"]);
    assert_eq!(on_alpha.as_array().unwrap().len(), 1, "{on_alpha}");
}

#[test]
fn a_name_already_in_use_is_refused() {
    let (alpha, beta) = (Machine::new("alpha", 3600), Machine::new("beta", 3600));
    let namesake = Machine::new("beta", 3600);
    let daemon = alpha.serve();
    let nowhere = closed_address();
    beta.join(&alpha, &daemon.address, &nowhere);

    let invite = alpha.pair(&daemon.address);
    namesake
        .recollect()
        .args(["join", &invite, "--address", &nowhere])
        .assert()
        .failure()
        .stderr(
            "error: alpha answered: a peer named \"beta\" already exists; remove it first with: recollect peer remove beta\n",
        );
    assert_eq!(
        namesake.json(&["peer", "list", "--json"]),
        serde_json::json!([])
    );
}

#[test]
fn a_malformed_invite_is_refused_before_anything_is_sent() {
    let beta = Machine::new("beta", 3600);
    beta.recollect()
        .args(["join", "alpha,127.0.0.1:1"])
        .assert()
        .failure()
        .stdout("")
        .stderr(
            "error: invalid invite: it must have four parts separated by commas; copy the whole invite\n",
        );
}

#[test]
fn an_unusable_address_is_refused() {
    let alpha = Machine::new("alpha", 3600);
    alpha
        .recollect()
        .args(["pair", "--address", "nowhere"])
        .assert()
        .failure()
        .stdout("")
        .stderr("error: invalid address \"nowhere\": use host:port, such as foehn:7327\n");
}

#[test]
fn three_machines_in_a_chain_never_bring_a_deleted_memory_back() {
    let (alpha, beta, gamma) = (
        Machine::new("alpha", 3600),
        Machine::new("beta", 3600),
        Machine::new("gamma", 3600),
    );
    // Only beta listens; alpha and gamma each know beta and not each other.
    let daemon = beta.serve();
    let nowhere = closed_address();
    alpha.join(&beta, &daemon.address, &nowhere);
    gamma.join(&beta, &daemon.address, &nowhere);
    alpha.store("doomed");
    alpha.sync("beta: sent 1 memory\n");
    gamma.sync("beta: received 1 memory\n");
    assert_eq!(gamma.contents(), ["doomed"]);

    alpha
        .recollect()
        .args(["delete", &alpha.id_of("doomed")])
        .assert()
        .success();
    alpha.sync("beta: sent 1 deletion\n");
    gamma.sync("beta: received 1 deletion\n");

    for machine in [&alpha, &beta, &gamma] {
        assert!(machine.contents().is_empty(), "{}", machine.name);
    }
    gamma.sync("beta: in sync\n");
    alpha.sync("beta: in sync\n");
}

#[test]
fn a_peer_that_removed_this_machine_says_so() {
    let (alpha, _alpha_daemon, beta, _beta_daemon) = paired();
    alpha
        .recollect()
        .args(["peer", "remove", "beta"])
        .assert()
        .success();

    beta.recollect()
        .arg("sync")
        .assert()
        .failure()
        .stdout(format!("alpha: error: alpha reports: {NOT_PAIRED}\n"))
        .stderr("error: sync failed for 1 of 1 peers\n");
    assert_eq!(
        beta.peer("alpha")["last_error"],
        format!("alpha reports: {NOT_PAIRED}")
    );
}

/// A machine played by the test, speaking the wire protocol through the
/// library's own transport.
fn stranger() -> Identity {
    let dir = tempfile::tempdir().unwrap();
    Identity::load_or_create(dir.path()).unwrap()
}

fn other_version_hello() -> Message {
    Message::Hello {
        protocol: PROTOCOL_VERSION + 1,
        manifest_hash: "whatever".into(),
    }
}

#[test]
fn sync_reports_a_peer_that_speaks_another_protocol_version() {
    let alpha = Machine::new("alpha", 3600);
    let gamma = stranger();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    alpha
        .database()
        .add_peer(
            "gamma",
            gamma.fingerprint(),
            &address,
            "2026-10-07T10:00:00.000Z",
        )
        .unwrap();
    let answering = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let mut connection = transport::accept(&gamma, socket, Timeouts::default()).unwrap();
        let hello = connection.channel.receive(MESSAGE_LIMIT).unwrap();
        connection.channel.send(&other_version_hello()).unwrap();
        hello
    });

    alpha
        .recollect()
        .arg("sync")
        .assert()
        .failure()
        .stdout(format!("gamma: error: {OTHER_PROTOCOL}\n"))
        .stderr("error: sync failed for 1 of 1 peers\n");

    let hello = answering.join().unwrap();
    assert!(
        matches!(
            hello,
            Message::Hello {
                protocol: PROTOCOL_VERSION,
                ..
            }
        ),
        "{hello:?}"
    );
    assert_eq!(alpha.peer("gamma")["last_error"], OTHER_PROTOCOL);
}

#[test]
fn the_daemon_refuses_a_peer_that_speaks_another_protocol_version() {
    let alpha = Machine::new("alpha", 3600);
    let gamma = stranger();
    // Started before the peer exists, so its first pass has nobody to dial.
    let daemon = alpha.serve();
    alpha
        .database()
        .add_peer(
            "gamma",
            gamma.fingerprint(),
            &closed_address(),
            "2026-10-07T10:00:00.000Z",
        )
        .unwrap();

    let mut connection = transport::connect(
        &gamma,
        &daemon.address,
        &alpha.fingerprint(),
        Timeouts::default(),
    )
    .unwrap();
    connection.channel.send(&other_version_hello()).unwrap();
    let answer = connection.channel.receive(MESSAGE_LIMIT).unwrap();

    assert!(
        matches!(
            answer,
            Message::Hello {
                protocol: PROTOCOL_VERSION,
                ..
            }
        ),
        "the peer learns alpha's version too: {answer:?}"
    );
    eventually("the refusal is recorded for the peer", || {
        alpha.peer("gamma")["last_error"] == OTHER_PROTOCOL
    });
    eventually("the refusal is logged", || {
        daemon
            .log()
            .contains(&format!("gamma: error: {OTHER_PROTOCOL}"))
    });
}

#[test]
fn sync_embeds_what_a_round_stored_before_it_failed() {
    let alpha = Machine::new("alpha", 3600);
    let gamma = stranger();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    alpha
        .database()
        .add_peer(
            "gamma",
            gamma.fingerprint(),
            &address,
            "2026-10-07T10:00:00.000Z",
        )
        .unwrap();
    let about_storage = "We keep every memory in a single SQLite file with a project column.";
    let memory = SyncRecord {
        global_id: "0199a8c0-0000-7000-8000-000000000001".into(),
        project: None,
        memory_type: MemoryType::Note,
        content: about_storage.into(),
        tags: Vec::new(),
        origin_peer: None,
        created_at: "2026-10-07T10:00:00.000Z".into(),
        deleted_at: None,
        deleted_by_peer: None,
    };
    // A mistake in the script fails the test instead of hanging it.
    let timeouts = Timeouts {
        io: Duration::from_secs(10),
        ..Timeouts::default()
    };
    // gamma, played by the test, hangs up where it should say what it applied.
    let answering = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let mut connection = transport::accept(&gamma, socket, timeouts).unwrap();
        let channel = &mut connection.channel;
        channel.receive(MESSAGE_LIMIT).unwrap();
        channel
            .send(&Message::Hello {
                protocol: PROTOCOL_VERSION,
                manifest_hash: "something else".into(),
            })
            .unwrap();
        channel.receive(MESSAGE_LIMIT).unwrap();
        channel
            .send(&Message::Manifest {
                entries: vec![(memory.global_id.clone(), EntryState::Live)],
            })
            .unwrap();
        while !matches!(channel.receive(MESSAGE_LIMIT).unwrap(), Message::End {}) {}
        channel
            .send(&Message::Records {
                memories: vec![memory],
            })
            .unwrap();
        channel
            .send(&Message::Tombstones {
                entries: Vec::new(),
            })
            .unwrap();
        channel.send(&Message::End {}).unwrap();
        channel.receive(MESSAGE_LIMIT).unwrap()
    });

    alpha
        .recollect()
        .arg("sync")
        .assert()
        .failure()
        .stdout("gamma: error: the peer closed the connection\n")
        .stderr("error: sync failed for 1 of 1 peers\n");

    let applied = answering.join().unwrap();
    assert_eq!(
        applied,
        Message::Applied {
            inserted: 1,
            deleted: 0
        }
    );
    assert_eq!(alpha.contents(), [about_storage]);
    assert_eq!(alpha.json(&["status", "--json"])["pending_embeddings"], 0);
    // No word of the query occurs in the memory: only vectors made on alpha find it.
    let found = alpha.json(&["search", "how is data persisted on disk", "--json"]);
    assert_eq!(found[0]["content"], about_storage);
}

#[test]
fn sync_names_an_unknown_peer_and_says_when_there_is_none() {
    let alpha = Machine::new("alpha", 3600);
    alpha.sync("no peers to sync with; pair one with recollect pair and recollect join\n");
    alpha
        .recollect()
        .args(["sync", "nobody"])
        .assert()
        .failure()
        .stdout("")
        .stderr("error: no peer named \"nobody\"\n");
}
