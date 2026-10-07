//! `recollect serve`: answers the machine's peers and syncs with them on a
//! timer.

use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::output::round_line;
use crate::service::Recollect;
use crate::sync::identity::Identity;
use crate::sync::protocol::UNPAIRED_MESSAGE_LIMIT;
use crate::sync::round::{self, RoundOutcome};
use crate::sync::transport::{self, Timeouts};
use crate::sync::{pairing, record_round, sync_with};
use crate::time::now_timestamp;

/// Where the daemon's log lines go.
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// How many incoming connections are answered at a time; further ones are
/// closed at once.
pub const MAX_CONNECTIONS: usize = 4;

/// Runs the sync daemon: answers the machine's peers, and syncs with every
/// peer now and then every `interval_seconds`. Returns only if it cannot
/// start; afterwards failures are logged and the daemon carries on.
pub fn serve(config: Config, log: Log) -> Result<()> {
    let identity = Arc::new(Identity::load_or_create(&config.data_dir)?);
    // Opened once here so that a database problem stops the daemon at
    // startup instead of failing every connection later.
    drop(Recollect::open(config.clone())?);
    let listen = config.sync.listen;
    let cannot_listen =
        |err: std::io::Error| Error::Sync(format!("cannot listen on {listen}: {err}"));
    let listener = TcpListener::bind(listen).map_err(cannot_listen)?;
    let address = listener.local_addr().map_err(cannot_listen)?;
    log(&format!("listening on {address}"));
    {
        let (config, identity, log) = (config.clone(), Arc::clone(&identity), Arc::clone(&log));
        std::thread::spawn(move || answer_peers(listener, config, identity, log));
    }
    let interval = Duration::from_secs(config.sync.interval_seconds);
    loop {
        if let Err(err) = sync_with_peers(&config, &identity, Timeouts::default(), &log) {
            log(&format!("error: {err}"));
        }
        std::thread::sleep(interval);
    }
}

/// Answers every incoming connection on a thread of its own, at most
/// `MAX_CONNECTIONS` at a time.
fn answer_peers(listener: TcpListener, config: Config, identity: Arc<Identity>, log: Log) {
    let answering = Arc::new(AtomicUsize::new(0));
    for socket in listener.incoming() {
        let Ok(socket) = socket else {
            // Out of file descriptors, most likely: give the system a moment.
            std::thread::sleep(Duration::from_millis(100));
            continue;
        };
        // Dropping the socket closes a connection there is no slot for.
        let Some(slot) = Slot::take(&answering) else {
            continue;
        };
        let (config, identity, log) = (config.clone(), Arc::clone(&identity), Arc::clone(&log));
        std::thread::spawn(move || {
            let _slot = slot;
            if let Err(err) = answer(&config, &identity, socket, Timeouts::default(), &log) {
                log(&format!("error: {err}"));
            }
        });
    }
}

/// One of the `MAX_CONNECTIONS` connections answered at a time. Dropping it
/// frees the slot, also when its thread panics.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(answering: &Arc<AtomicUsize>) -> Option<Self> {
        if answering.fetch_add(1, Ordering::SeqCst) < MAX_CONNECTIONS {
            Some(Self(Arc::clone(answering)))
        } else {
            answering.fetch_sub(1, Ordering::SeqCst);
            None
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Opens the machine's database for the daemon's work. Notices about that
/// work, such as the start of a model download, go to the daemon's log.
fn open_app(config: &Config, log: &Log) -> Result<Recollect> {
    let log = Arc::clone(log);
    Ok(Recollect::open(config.clone())?.with_notices(move |notice| log(notice)))
}

/// Answers one connection: a peer gets a sync round; a machine that is not a
/// peer may redeem an invite and nothing else.
fn answer(
    config: &Config,
    identity: &Identity,
    socket: TcpStream,
    timeouts: Timeouts,
    log: &Log,
) -> Result<()> {
    // Whatever cannot finish the handshake (a port scanner, a browser) is
    // dropped without a log line.
    let Ok(mut connection) = transport::accept(identity, socket, timeouts) else {
        return Ok(());
    };
    let fingerprint = connection.peer_fingerprint.clone();
    let mut app = open_app(config, log)?;
    match app.db().peer_with_fingerprint(&fingerprint)? {
        Some(peer) => {
            let result = round::respond(&mut connection.channel, app.db_mut(), &peer.name);
            // Hung up before the embedding starts, which can take a while.
            drop(connection);
            let previous_error = match &result {
                // A round the peer started says nothing about whether this
                // machine can reach the peer: the failure of this machine's
                // own last round stays recorded. So the answered round is
                // not logged as the peer being back, and the next failed
                // dial is not logged as a new failure.
                Ok(_) => {
                    app.db_mut()
                        .record_answered_round(&peer.name, &now_timestamp())?;
                    None
                }
                Err(_) => record_round(app.db_mut(), &peer.name, &result)?,
            };
            finish_round(
                &mut app,
                &peer.name,
                &result,
                previous_error.as_deref(),
                log,
            )
        }
        None => {
            let paired = connection
                .channel
                .receive(UNPAIRED_MESSAGE_LIMIT)
                .and_then(|message| {
                    pairing::accept(
                        &mut connection.channel,
                        app.db_mut(),
                        &fingerprint,
                        message,
                        Utc::now(),
                    )
                });
            match paired {
                Ok(name) => log(&format!("paired with {name}")),
                Err(err) => log(&format!(
                    "refused a machine that is not a peer ({fingerprint}): {err}"
                )),
            }
            Ok(())
        }
    }
}

/// Runs one round with every peer, one after the other. The model a round's
/// memories need is loaded at most once and dropped with `app` at the end,
/// so an idle daemon stays small.
fn sync_with_peers(
    config: &Config,
    identity: &Identity,
    timeouts: Timeouts,
    log: &Log,
) -> Result<()> {
    let mut app = open_app(config, log)?;
    for peer in app.db().peers()? {
        let report = sync_with(&mut app, identity, &peer, timeouts)?;
        finish_round(
            &mut app,
            &report.peer,
            &report.result,
            report.previous_error.as_deref(),
            log,
        )?;
    }
    Ok(())
}

/// Logs a round if it is worth a line and embeds the memories it brought,
/// also when it failed after storing them.
fn finish_round(
    app: &mut Recollect,
    peer: &str,
    result: &Result<RoundOutcome>,
    previous_error: Option<&str>,
    log: &Log,
) -> Result<()> {
    if let Some(line) = round_log_line(peer, result, previous_error) {
        log(&line);
    }
    if round::received_memories(result) > 0
        && let Some(warning) = app.embed_received()?
    {
        log(&format!("warning: {warning}"));
    }
    Ok(())
}

/// The line to log for a round, or `None` when it brings nothing new: a
/// round that changed nothing, or a failure that was logged already. A
/// sleeping laptop is normal and must not fill the journal.
fn round_log_line(
    peer: &str,
    result: &Result<RoundOutcome>,
    previous_error: Option<&str>,
) -> Option<String> {
    match result {
        Ok(outcome) if *outcome != RoundOutcome::default() => Some(round_line(peer, result)),
        Ok(_) => previous_error.map(|_| format!("{peer}: syncing again")),
        Err(err) if previous_error == Some(err.to_string().as_str()) => None,
        Err(_) => Some(round_line(peer, result)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::Mutex;

    use super::*;
    use crate::db::test_support::note;
    use crate::sync::exchange::manifest;
    use crate::sync::pairing::{self, NOT_PAIRED};
    use crate::sync::protocol::{MESSAGE_LIMIT, Message, PROTOCOL_VERSION};
    use crate::sync::test_support::{Machine, closed_address, listener};

    fn collecting_log() -> (Log, Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let log: Log = Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string()));
        (log, lines)
    }

    /// Lets `machine` answer the next connection on `listener` while `visit`
    /// runs; returns what `visit` returns and the lines the machine logged.
    fn answering<T>(
        machine: &Machine,
        listener: &TcpListener,
        visit: impl FnOnce() -> T,
    ) -> (T, Vec<String>) {
        let (log, lines) = collecting_log();
        let visited = std::thread::scope(|scope| {
            scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                answer(
                    &machine.config(),
                    &machine.identity,
                    socket,
                    Timeouts::default(),
                    &log,
                )
                .unwrap();
            });
            visit()
        });
        let lines = lines.lock().unwrap().clone();
        (visited, lines)
    }

    /// The notice that precedes the attempt to load a model that is not there.
    const DOWNLOADING: &str = "downloading embedding model ";

    const NO_VECTORS: &str = "warning: stored 1 memories without embedding: ";

    #[test]
    fn a_peers_round_is_answered_recorded_and_logged() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        b.knows("a", &a, "a.example:7327");
        let mut a_db = a.database();
        a_db.insert_memory(&note("from a"), None).unwrap();

        let (initiated, lines) = answering(&b, &listener, || {
            let mut connection = a.connect_to(&b, &address);
            round::initiate(&mut connection.channel, &mut a_db, "b")
        });

        assert_eq!(initiated.unwrap().sent.memories, 1);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "a: received 1 memory");
        assert!(lines[1].starts_with(DOWNLOADING), "{lines:?}");
        assert!(lines[2].starts_with(NO_VECTORS), "{lines:?}");
        let b_db = b.database();
        assert_eq!(b_db.live_count().unwrap(), 1);
        let recorded = b_db.peer_named("a").unwrap();
        assert!(recorded.last_sync_at.is_some());
        assert_eq!(recorded.last_error, None);
    }

    #[test]
    fn a_round_that_changes_nothing_is_not_logged() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        b.knows("a", &a, "a.example:7327");
        let mut a_db = a.database();

        let (initiated, lines) = answering(&b, &listener, || {
            let mut connection = a.connect_to(&b, &address);
            round::initiate(&mut connection.channel, &mut a_db, "b")
        });

        assert_eq!(initiated.unwrap(), RoundOutcome::default());
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn what_a_round_stored_before_it_failed_is_embedded_too() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        b.knows("a", &a, "a.example:7327");
        let mut a_db = a.database();
        a_db.insert_memory(&note("from a"), None).unwrap();
        let rows = a_db.sync_manifest().unwrap();
        let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
        let (entries, memories) = (manifest(rows), a_db.sync_records(&ids).unwrap());

        // a, played by the test, hangs up where it should say what it applied.
        let ((), lines) = answering(&b, &listener, || {
            let mut connection = a.connect_to(&b, &address);
            let channel = &mut connection.channel;
            channel
                .send(&Message::Hello {
                    protocol: PROTOCOL_VERSION,
                    manifest_hash: "something else".into(),
                })
                .unwrap();
            channel.receive(MESSAGE_LIMIT).unwrap();
            channel.send(&Message::Manifest { entries }).unwrap();
            channel.receive(MESSAGE_LIMIT).unwrap();
            channel.send(&Message::Records { memories }).unwrap();
            channel
                .send(&Message::Tombstones {
                    entries: Vec::new(),
                })
                .unwrap();
            channel.send(&Message::End {}).unwrap();
            while !matches!(channel.receive(MESSAGE_LIMIT).unwrap(), Message::End {}) {}
        });

        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "a: error: the peer closed the connection");
        assert!(lines[1].starts_with(DOWNLOADING), "{lines:?}");
        assert!(lines[2].starts_with(NO_VECTORS), "{lines:?}");
        let b_db = b.database();
        assert_eq!(b_db.live_count().unwrap(), 1);
        let recorded = b_db.peer_named("a").unwrap();
        assert_eq!(
            recorded.last_error.as_deref(),
            Some("the peer closed the connection")
        );
        assert_eq!(recorded.last_sync_at, None);
    }

    #[test]
    fn a_machine_with_a_valid_invite_becomes_a_peer() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        let invite = pairing::create_invite(
            &mut b.database(),
            &b.identity,
            "b",
            &address,
            chrono::Utc::now(),
        )
        .unwrap();
        let mut a_db = a.database();

        let (joined, lines) = answering(&b, &listener, || {
            pairing::join(
                &mut a_db,
                &a.identity,
                &invite,
                "a",
                "a.example:7327",
                Timeouts::default(),
            )
        });

        assert_eq!(joined.unwrap().name, "b");
        assert_eq!(lines, ["paired with a"]);
        assert_eq!(
            b.database().peer_named("a").unwrap().fingerprint,
            a.identity.fingerprint()
        );
    }

    #[test]
    fn an_unknown_machine_without_an_invite_is_told_it_is_not_paired() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();

        let (reply, lines) = answering(&b, &listener, || {
            let mut connection = a.connect_to(&b, &address);
            connection
                .channel
                .send(&Message::Hello {
                    protocol: PROTOCOL_VERSION,
                    manifest_hash: "abc".into(),
                })
                .unwrap();
            connection.channel.receive(MESSAGE_LIMIT).unwrap()
        });

        assert_eq!(
            reply,
            Message::Error {
                message: NOT_PAIRED.into()
            }
        );
        assert_eq!(
            lines,
            [format!(
                "refused a machine that is not a peer ({}): {NOT_PAIRED}",
                a.identity.fingerprint()
            )]
        );
    }

    #[test]
    fn an_unknown_machine_may_not_send_more_than_a_small_message() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();

        let (reply, lines) = answering(&b, &listener, || {
            let mut connection = a.connect_to(&b, &address);
            connection
                .channel
                .send(&Message::Pair {
                    protocol: PROTOCOL_VERSION,
                    secret: "x".repeat(5000),
                    name: "a".into(),
                    address: "a.example:7327".into(),
                })
                .unwrap();
            connection.channel.receive(MESSAGE_LIMIT)
        });

        assert!(reply.is_err(), "the connection is closed without an answer");
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].ends_with("; the limit is 4096"), "{lines:?}");
        assert!(b.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_connection_that_does_not_speak_tls_is_dropped_without_a_word() {
        let b = Machine::new();
        let (listener, address) = listener();

        let ((), lines) = answering(&b, &listener, || {
            let mut socket = TcpStream::connect(&address).unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        });

        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn a_pass_syncs_with_every_peer_and_logs_a_failure_only_once() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        let nowhere = closed_address();
        a.knows("b", &b, &address);
        a.knows("c", &Machine::new(), &nowhere);
        let mut b_db = b.database();
        b_db.insert_memory(&note("from b"), None).unwrap();
        let (log, lines) = collecting_log();

        // b answers two rounds, the way its daemon would.
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..2 {
                    let (socket, _) = listener.accept().unwrap();
                    let mut connection =
                        transport::accept(&b.identity, socket, Timeouts::default()).unwrap();
                    round::respond(&mut connection.channel, &mut b_db, "a").unwrap();
                }
            });
            sync_with_peers(&a.config(), &a.identity, Timeouts::default(), &log).unwrap();
            sync_with_peers(&a.config(), &a.identity, Timeouts::default(), &log).unwrap();
        });

        let lines = lines.lock().unwrap().clone();
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(lines[0], "b: received 1 memory");
        assert!(lines[1].starts_with(DOWNLOADING), "{lines:?}");
        assert!(lines[2].starts_with(NO_VECTORS), "{lines:?}");
        assert!(
            lines[3].starts_with(&format!("c: error: {nowhere}: cannot connect (")),
            "the second pass repeats neither the failure nor the quiet round: {lines:?}"
        );
        assert_eq!(a.database().live_count().unwrap(), 1);
    }

    #[test]
    fn a_peer_that_reaches_this_machine_but_cannot_be_dialled_is_logged_once() {
        let (a, b) = (Machine::new(), Machine::new());
        let (listener, address) = listener();
        let nowhere = closed_address();
        a.knows("b", &b, &nowhere);
        let mut b_db = b.database();
        b_db.insert_memory(&note("from b"), None).unwrap();
        let (log, logged_by_passes) = collecting_log();
        let mut lines = Vec::new();

        // Three intervals: a's dial fails, then b's round with a succeeds.
        // Only the first of b's rounds brings something.
        for _ in 0..3 {
            sync_with_peers(&a.config(), &a.identity, Timeouts::default(), &log).unwrap();
            lines.append(&mut logged_by_passes.lock().unwrap());
            let (initiated, logged_by_answer) = answering(&a, &listener, || {
                let mut connection = b.connect_to(&a, &address);
                round::initiate(&mut connection.channel, &mut b_db, "a")
            });
            initiated.unwrap();
            lines.extend(logged_by_answer);
        }

        assert_eq!(
            lines.len(),
            4,
            "an answered round neither ends nor renews the dial failure: {lines:?}"
        );
        let dial_failure = lines[0]
            .strip_prefix("b: error: ")
            .expect("the dial failure");
        assert!(
            dial_failure.starts_with(&format!("{nowhere}: cannot connect (")),
            "{lines:?}"
        );
        assert_eq!(lines[1], "b: received 1 memory");
        assert!(lines[2].starts_with(DOWNLOADING), "{lines:?}");
        assert!(lines[3].starts_with(NO_VECTORS), "{lines:?}");
        let recorded = a.database().peer_named("b").unwrap();
        assert!(recorded.last_sync_at.is_some());
        assert_eq!(recorded.last_error.as_deref(), Some(dial_failure));
    }

    #[test]
    fn only_what_is_new_about_a_peer_is_logged() {
        let synced = |received: usize| -> Result<RoundOutcome> {
            let mut outcome = RoundOutcome::default();
            outcome.received.memories = received;
            Ok(outcome)
        };
        let failed = |reason: &str| -> Result<RoundOutcome> { Err(Error::Sync(reason.into())) };
        for (result, previous_error, expected) in [
            (synced(0), None, None),
            (synced(2), None, Some("b: received 2 memories")),
            (synced(0), Some("down"), Some("b: syncing again")),
            (synced(2), Some("down"), Some("b: received 2 memories")),
            (failed("down"), None, Some("b: error: down")),
            (failed("down"), Some("down"), None),
            (failed("refused"), Some("down"), Some("b: error: refused")),
        ] {
            assert_eq!(
                round_log_line("b", &result, previous_error).as_deref(),
                expected,
                "{result:?} after {previous_error:?}"
            );
        }
    }

    #[test]
    fn at_most_four_connections_are_answered_at_a_time() {
        let answering = Arc::new(AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..MAX_CONNECTIONS)
            .map(|_| Slot::take(&answering).expect("a free slot"))
            .collect();
        assert!(Slot::take(&answering).is_none(), "the fifth is turned away");
        drop(slots);
        assert!(
            Slot::take(&answering).is_some(),
            "finished connections free their slots"
        );
    }
}
