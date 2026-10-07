//! Sync between machines: paired peers exchange their memories and deletions
//! directly, with no server in between.

pub mod daemon;
pub mod exchange;
pub mod identity;
pub mod pairing;
pub mod protocol;
pub mod round;
pub mod transport;

use serde::Serialize;

use crate::config::Config;
use crate::db::{Database, Peer};
use crate::error::Result;
use crate::service::Recollect;
use crate::sync::identity::{Identity, local_name};
use crate::sync::round::RoundOutcome;
use crate::sync::transport::Timeouts;
use crate::time::now_timestamp;

/// Whether `name` can label a machine: 1 to 64 ASCII letters, digits, `.`,
/// `_` or `-`.
pub fn is_valid_peer_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Whether `address` is a `host:port` another machine can dial: a host
/// without whitespace or commas (an invite separates its parts with commas)
/// and a port from 1 to 65535.
pub fn is_valid_address(address: &str) -> bool {
    match address.rsplit_once(':') {
        Some((host, port)) => {
            !host.is_empty()
                && !host.contains(|c: char| c.is_whitespace() || c == ',')
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        }
        None => false,
    }
}

/// The SHA-256 of `bytes` in lowercase hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// How a round with one peer went.
pub struct SyncReport {
    /// The peer's name.
    pub peer: String,
    pub result: Result<RoundOutcome>,
    /// The failure recorded for the peer before this round, which tells a
    /// new failure from a repeated one.
    pub previous_error: Option<String>,
}

/// Records how a round with `peer` went; returns the failure recorded before.
pub(crate) fn record_round(
    db: &mut Database,
    peer: &str,
    result: &Result<RoundOutcome>,
) -> Result<Option<String>> {
    let failure = result.as_ref().err().map(|err| err.to_string());
    db.record_round(peer, &now_timestamp(), failure.as_deref())
}

/// Dials `peer`, runs one round with it and records how it went. A round
/// that fails is part of the report; only a failure to record it is an error.
pub fn sync_with(
    app: &mut Recollect,
    identity: &Identity,
    peer: &Peer,
    timeouts: Timeouts,
) -> Result<SyncReport> {
    let result = transport::connect(identity, &peer.address, &peer.fingerprint, timeouts).and_then(
        |mut connection| round::initiate(&mut connection.channel, app.db_mut(), &peer.name),
    );
    let previous_error = record_round(app.db_mut(), &peer.name, &result)?;
    Ok(SyncReport {
        peer: peer.name.clone(),
        result,
        previous_error,
    })
}

/// What `recollect id` shows: how this machine appears to its peers.
#[derive(Debug, PartialEq, Serialize)]
pub struct LocalMachine {
    pub name: String,
    pub fingerprint: String,
    /// The address and port `recollect serve` listens on.
    pub listen: String,
}

/// This machine's name, key fingerprint (the key is generated on first use)
/// and listen address.
pub fn local_machine(config: &Config) -> Result<LocalMachine> {
    Ok(LocalMachine {
        name: local_name(config)?,
        fingerprint: Identity::load_or_create(&config.data_dir)?
            .fingerprint()
            .to_string(),
        listen: config.sync.listen.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;
    use crate::config::Config;
    use crate::db::Database;
    use crate::db::test_support::note;
    use crate::service::Recollect;
    use crate::sync::identity::Identity;
    use crate::sync::transport::Timeouts;

    const T0: &str = "2026-10-07T10:00:00.000Z";

    /// A machine with a data directory: its key and its database.
    struct Machine {
        dir: tempfile::TempDir,
        identity: Identity,
    }

    impl Machine {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let identity = Identity::load_or_create(dir.path()).unwrap();
            Self { dir, identity }
        }

        /// The machine as the commands open it; the model is never loaded here.
        fn app(&self) -> Recollect {
            let config = Config::load_from(
                self.dir.path().to_path_buf(),
                Some(self.dir.path().join("no-model")),
            )
            .unwrap();
            Recollect::open(config).unwrap()
        }

        fn database(&self) -> Database {
            Database::open(&self.dir.path().join("memories.db")).unwrap()
        }
    }

    #[test]
    fn peer_names_are_short_and_made_of_host_name_characters() {
        let too_long = "x".repeat(65);
        for name in ["foehn", "twelve", "Work-Laptop_2", "a.b"] {
            assert!(is_valid_peer_name(name), "{name}");
        }
        for name in ["", "two words", "a,b", "ä", too_long.as_str()] {
            assert!(!is_valid_peer_name(name), "{name}");
        }
    }

    #[test]
    fn addresses_are_a_host_and_a_port() {
        for address in [
            "foehn:7327",
            "192.168.1.5:7327",
            "[::1]:7327",
            "foehn.local:1",
        ] {
            assert!(is_valid_address(address), "{address}");
        }
        for address in [
            "",
            "foehn",
            "foehn:",
            ":7327",
            "foehn:0",
            "foehn:70000",
            "fo ehn:7327",
            "a,b:7327",
        ] {
            assert!(!is_valid_address(address), "{address}");
        }
    }

    #[test]
    fn a_round_with_a_peer_is_recorded_as_its_last_sync() {
        let (a, b) = (Machine::new(), Machine::new());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let mut app = a.app();
        app.db_mut()
            .add_peer("b", b.identity.fingerprint(), &address, T0)
            .unwrap();
        let mut b_db = b.database();
        b_db.insert_memory(&note("from b"), None).unwrap();

        let report = std::thread::scope(|scope| {
            scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                let mut connection =
                    transport::accept(&b.identity, socket, Timeouts::default()).unwrap();
                assert_eq!(connection.peer_fingerprint, a.identity.fingerprint());
                round::respond(&mut connection.channel, &mut b_db, "a").unwrap();
            });
            let peer = app.db_mut().peer_named("b").unwrap();
            sync_with(&mut app, &a.identity, &peer, Timeouts::default()).unwrap()
        });

        assert_eq!(report.peer, "b");
        assert_eq!(report.result.unwrap().received.memories, 1);
        assert_eq!(report.previous_error, None);
        let peer = app.db_mut().peer_named("b").unwrap();
        assert!(peer.last_sync_at.is_some());
        assert_eq!(peer.last_error, None);
    }

    #[test]
    fn an_unreachable_peer_is_recorded_with_the_reason_and_the_one_before() {
        let (a, b) = (Machine::new(), Machine::new());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        drop(listener);
        let mut app = a.app();
        app.db_mut()
            .add_peer("b", b.identity.fingerprint(), &address, T0)
            .unwrap();
        let peer = app.db_mut().peer_named("b").unwrap();

        let first = sync_with(&mut app, &a.identity, &peer, Timeouts::default()).unwrap();
        let reason = first.result.unwrap_err().to_string();
        assert!(
            reason.starts_with(&format!("{address}: cannot connect (")),
            "{reason}"
        );
        assert_eq!(first.previous_error, None);
        let stored = app.db_mut().peer_named("b").unwrap();
        assert_eq!(stored.last_error.as_deref(), Some(reason.as_str()));
        assert_eq!(stored.last_sync_at, None);

        let second = sync_with(&mut app, &a.identity, &peer, Timeouts::default()).unwrap();
        assert_eq!(second.previous_error.as_deref(), Some(reason.as_str()));
    }
}
