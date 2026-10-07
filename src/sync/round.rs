//! One sync round between two machines over an open channel.

use std::io::{Read, Write};

use crate::db::{Database, SyncApplied};
use crate::error::{Error, Result};
use crate::memory::{SyncRecord, Tombstone};
use crate::sync::exchange::{batches, manifest, manifest_hash, outgoing, validate};
use crate::sync::protocol::{
    Channel, MESSAGE_LIMIT, ManifestEntry, Message, PROTOCOL_VERSION, printable,
};

/// What one round changed on both machines.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RoundOutcome {
    /// What this machine applied.
    pub received: SyncApplied,
    /// What the peer reports having applied.
    pub sent: SyncApplied,
}

/// The live memories this machine stored in a round and now has to embed,
/// also when the round failed after storing them.
pub fn received_memories(result: &Result<RoundOutcome>) -> usize {
    match result {
        Ok(outcome) => outcome.received.memories,
        Err(Error::RoundFailedAfterApply { received, .. }) => *received,
        Err(_) => 0,
    }
}

/// What this machine holds, read once when the round starts.
struct Holdings {
    entries: Vec<ManifestEntry>,
    hash: String,
}

impl Holdings {
    fn of(db: &Database) -> Result<Self> {
        let entries = manifest(db.sync_manifest()?);
        let hash = manifest_hash(&entries);
        Ok(Self { entries, hash })
    }

    fn hello(&self) -> Message {
        Message::Hello {
            protocol: PROTOCOL_VERSION,
            manifest_hash: self.hash.clone(),
        }
    }

    fn manifest(&self) -> Message {
        Message::Manifest {
            entries: self.entries.clone(),
        }
    }
}

/// Runs a round as the machine that connected to `peer`. The two sides send
/// strictly in turn, so neither ever waits on a full socket buffer.
pub fn initiate<S: Read + Write>(
    channel: &mut Channel<S>,
    db: &mut Database,
    peer: &str,
) -> Result<RoundOutcome> {
    let mine = Holdings::of(db)?;
    channel.send(&mine.hello())?;
    let their_hash = read_hello(receive(channel, peer)?, peer)?;
    if their_hash == mine.hash {
        return Ok(RoundOutcome::default());
    }
    channel.send(&mine.manifest())?;
    let theirs = read_manifest(receive(channel, peer)?, peer)?;
    send_changes(channel, db, &mine.entries, &theirs)?;
    let received = receive_and_apply(channel, db, peer)?;
    let sent = after_apply(received, || {
        channel.send(&applied_message(received))?;
        read_applied(receive(channel, peer)?, peer)
    })?;
    Ok(RoundOutcome { received, sent })
}

/// Runs a round as the machine `peer` connected to.
pub fn respond<S: Read + Write>(
    channel: &mut Channel<S>,
    db: &mut Database,
    peer: &str,
) -> Result<RoundOutcome> {
    let mine = Holdings::of(db)?;
    let their_hello = receive(channel, peer)?;
    // Answered before it is checked, so that a peer with another protocol
    // version learns this machine's version too.
    channel.send(&mine.hello())?;
    let their_hash = read_hello(their_hello, peer)?;
    if their_hash == mine.hash {
        return Ok(RoundOutcome::default());
    }
    let theirs = read_manifest(receive(channel, peer)?, peer)?;
    channel.send(&mine.manifest())?;
    let received = receive_and_apply(channel, db, peer)?;
    let sent = after_apply(received, || {
        send_changes(channel, db, &mine.entries, &theirs)?;
        let sent = read_applied(receive(channel, peer)?, peer)?;
        channel.send(&applied_message(received))?;
        Ok(sent)
    })?;
    Ok(RoundOutcome { received, sent })
}

/// Runs the steps of a round that follow this machine's apply. A failure in
/// them still says what was stored, so that the caller can embed it.
fn after_apply<T>(received: SyncApplied, steps: impl FnOnce() -> Result<T>) -> Result<T> {
    steps().map_err(|source| Error::RoundFailedAfterApply {
        received: received.memories,
        source: Box::new(source),
    })
}

/// The next message from `peer`; an `error` message from it ends the round.
fn receive<S: Read + Write>(channel: &mut Channel<S>, peer: &str) -> Result<Message> {
    match channel.receive(MESSAGE_LIMIT)? {
        Message::Error { message } => Err(Error::Sync(format!(
            "{peer} reports: {}",
            printable(&message)
        ))),
        message => Ok(message),
    }
}

fn out_of_turn(peer: &str, expected: &str, message: &Message) -> Error {
    Error::Sync(format!(
        "{peer} sent {} where {expected} was expected",
        message.name()
    ))
}

/// The manifest hash of a `hello` in this machine's protocol version.
fn read_hello(message: Message, peer: &str) -> Result<String> {
    match message {
        Message::Hello {
            protocol,
            manifest_hash,
        } if protocol == PROTOCOL_VERSION => Ok(manifest_hash),
        Message::Hello { protocol, .. } => Err(Error::ProtocolMismatch {
            peer: peer.to_string(),
            theirs: protocol,
            ours: PROTOCOL_VERSION,
        }),
        other => Err(out_of_turn(peer, "hello", &other)),
    }
}

fn read_manifest(message: Message, peer: &str) -> Result<Vec<ManifestEntry>> {
    match message {
        Message::Manifest { entries } => Ok(entries),
        other => Err(out_of_turn(peer, "manifest", &other)),
    }
}

fn read_applied(message: Message, peer: &str) -> Result<SyncApplied> {
    match message {
        Message::Applied { inserted, deleted } => Ok(SyncApplied {
            memories: inserted,
            deletions: deleted,
        }),
        other => Err(out_of_turn(peer, "applied", &other)),
    }
}

fn applied_message(applied: SyncApplied) -> Message {
    Message::Applied {
        inserted: applied.memories,
        deleted: applied.deletions,
    }
}

/// Sends what the peer lacks: in full the memories it has never seen, live
/// or deleted, then the deletions of memories it still holds live, then `end`.
fn send_changes<S: Read + Write>(
    channel: &mut Channel<S>,
    db: &Database,
    mine: &[ManifestEntry],
    theirs: &[ManifestEntry],
) -> Result<()> {
    let owed = outgoing(mine, theirs);
    for memories in batches(db.sync_records(&owed.records)?) {
        channel.send(&Message::Records { memories })?;
    }
    channel.send(&Message::Tombstones {
        entries: db.sync_tombstones(&owed.tombstones)?,
    })?;
    channel.send(&Message::End {})
}

/// Reads the peer's changes up to `end`, checks them and applies them in one
/// transaction. A refusal is reported to the peer before the round ends.
fn receive_and_apply<S: Read + Write>(
    channel: &mut Channel<S>,
    db: &mut Database,
    peer: &str,
) -> Result<SyncApplied> {
    let mut records: Vec<SyncRecord> = Vec::new();
    let mut tombstones: Vec<Tombstone> = Vec::new();
    loop {
        match receive(channel, peer)? {
            Message::Records { memories } => records.extend(memories),
            Message::Tombstones { entries } => tombstones.extend(entries),
            Message::End {} => break,
            other => return Err(out_of_turn(peer, "records, tombstones or end", &other)),
        }
    }
    // The peer is told why the round ends, if it is still there to hear it.
    if let Err(err) = validate(&records, &tombstones) {
        let _ = channel.send(&Message::Error {
            message: err.to_string(),
        });
        return Err(Error::Sync(format!("{peer} sent an {err}")));
    }
    let applied = db.apply_sync(&records, &tombstones);
    if let Err(err) = &applied {
        let _ = channel.send(&Message::Error {
            message: format!("the changes could not be stored: {err}"),
        });
    }
    applied
}

#[cfg(test)]
mod tests {
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    use super::*;
    use crate::db::test_support::{note, record};
    use crate::filter::Filter;
    use crate::time::now_timestamp;

    fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let connecting = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (answering, _) = listener.accept().unwrap();
        (connecting, answering)
    }

    /// Runs one round over a real socket, `a` initiating and `b` responding
    /// on a thread of its own; returns what each side reports.
    fn round(a: &mut Database, b: &mut Database) -> (Result<RoundOutcome>, Result<RoundOutcome>) {
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            let responder = scope.spawn(move || respond(&mut Channel::new(answering), b, "a"));
            let initiated = initiate(&mut Channel::new(connecting), a, "b");
            (initiated, responder.join().unwrap())
        })
    }

    fn database() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn store(db: &mut Database, content: &str) -> i64 {
        db.insert_memory(&note(content), None).unwrap()
    }

    /// The contents of the live memories, sorted.
    fn contents(db: &Database) -> Vec<String> {
        let mut contents: Vec<String> = db
            .list(&Filter::default(), 10_000)
            .unwrap()
            .into_iter()
            .map(|memory| memory.content)
            .collect();
        contents.sort();
        contents
    }

    /// A memory as a peer sends it: the manifest that lists it and its record.
    fn memory_to_send(content: &str) -> (Vec<ManifestEntry>, Vec<SyncRecord>) {
        let mut scratch = database();
        store(&mut scratch, content);
        let rows = scratch.sync_manifest().unwrap();
        let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
        (manifest(rows), scratch.sync_records(&ids).unwrap())
    }

    /// A scripted peer's channel: it fails the test rather than hang it.
    fn scripted(socket: TcpStream) -> Channel<TcpStream> {
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Channel::new(socket)
    }

    /// Reads what the peer sends up to and including its `end`.
    fn receive_until_end(channel: &mut Channel<TcpStream>) {
        while !matches!(channel.receive(MESSAGE_LIMIT).unwrap(), Message::End {}) {}
    }

    fn applied(memories: usize, deletions: usize) -> SyncApplied {
        SyncApplied {
            memories,
            deletions,
        }
    }

    #[test]
    fn a_round_leaves_both_sides_with_every_memory() {
        let (mut a, mut b) = (database(), database());
        store(&mut a, "from a");
        store(&mut b, "from b 1");
        store(&mut b, "from b 2");

        let (initiated, responded) = round(&mut a, &mut b);

        assert_eq!(received_memories(&initiated), 2);
        assert_eq!(received_memories(&responded), 1);
        assert_eq!(
            initiated.unwrap(),
            RoundOutcome {
                received: applied(2, 0),
                sent: applied(1, 0)
            }
        );
        assert_eq!(
            responded.unwrap(),
            RoundOutcome {
                received: applied(1, 0),
                sent: applied(2, 0)
            }
        );
        let everything = ["from a", "from b 1", "from b 2"];
        assert_eq!(contents(&a), everything);
        assert_eq!(contents(&b), everything);
        assert_eq!(a.sync_manifest().unwrap(), b.sync_manifest().unwrap());
        assert_eq!(
            a.pending_embedding_count().unwrap(),
            3,
            "vectors never travel"
        );
    }

    #[test]
    fn machines_that_hold_the_same_stop_after_the_hello() {
        let (mut a, mut b) = (database(), database());
        store(&mut a, "shared");
        round(&mut a, &mut b).0.unwrap();

        let (initiated, responded) = round(&mut a, &mut b);

        assert_eq!(initiated.unwrap(), RoundOutcome::default());
        assert_eq!(responded.unwrap(), RoundOutcome::default());
    }

    #[test]
    fn deletions_travel_in_both_directions() {
        let (mut a, mut b) = (database(), database());
        let on_a = store(&mut a, "deleted on a");
        store(&mut a, "kept");
        round(&mut a, &mut b).0.unwrap();
        let on_b = b
            .list(&Filter::default(), 10)
            .unwrap()
            .into_iter()
            .find(|memory| memory.content == "kept")
            .unwrap()
            .id;
        a.delete(on_a, &now_timestamp()).unwrap();
        b.delete(on_b, &now_timestamp()).unwrap();

        let (initiated, _) = round(&mut a, &mut b);

        assert_eq!(
            initiated.unwrap(),
            RoundOutcome {
                received: applied(0, 1),
                sent: applied(0, 1)
            }
        );
        assert!(contents(&a).is_empty());
        assert!(contents(&b).is_empty());
        assert_eq!(a.sync_manifest().unwrap(), b.sync_manifest().unwrap());
    }

    #[test]
    fn a_memory_deleted_before_a_machine_saw_it_never_comes_back_through_a_third() {
        let (mut a, mut b, mut c) = (database(), database(), database());
        let doomed = store(&mut a, "doomed");
        round(&mut a, &mut c).0.unwrap();
        assert_eq!(contents(&c), ["doomed"]);
        a.delete(doomed, &now_timestamp()).unwrap();

        // b has never seen the memory: it arrives as a tombstone.
        round(&mut a, &mut b).0.unwrap();
        assert!(contents(&b).is_empty());
        assert_eq!(b.sync_manifest().unwrap().len(), 1);

        // c still holds it live: b passes the deletion on instead of taking it back.
        round(&mut b, &mut c).0.unwrap();
        assert!(contents(&c).is_empty());
        assert!(contents(&b).is_empty());
        round(&mut c, &mut a).0.unwrap();
        assert!(contents(&a).is_empty());
    }

    #[test]
    fn many_memories_travel_in_several_messages() {
        let (mut a, mut b) = (database(), database());
        for n in 0..450 {
            store(&mut a, &format!("memory {n}"));
        }
        let (initiated, responded) = round(&mut a, &mut b);
        assert_eq!(initiated.unwrap().sent, applied(450, 0));
        assert_eq!(responded.unwrap().received, applied(450, 0));
        assert_eq!(contents(&b).len(), 450);
    }

    #[test]
    fn an_invalid_record_is_refused_and_nothing_is_applied_on_either_side() {
        let (mut a, mut b) = (database(), database());
        // A global id no store would generate.
        a.insert_memory(&record("odd", Some("p"), "2026-09-01T10:00:00.000Z"), None)
            .unwrap();
        store(&mut a, "fine");
        store(&mut b, "from b");

        let (initiated, responded) = round(&mut a, &mut b);

        assert_eq!(received_memories(&initiated), 0);
        assert_eq!(received_memories(&responded), 0);
        assert_eq!(
            responded.unwrap_err().to_string(),
            "a sent an invalid record \"test-odd\": its global id is not a UUID"
        );
        assert_eq!(
            initiated.unwrap_err().to_string(),
            "b reports: invalid record \"test-odd\": its global id is not a UUID"
        );
        assert_eq!(contents(&b), ["from b"]);
        assert_eq!(contents(&a), ["fine", "odd"]);
    }

    #[test]
    fn the_responder_refuses_a_peer_with_another_protocol_version() {
        let mut b = database();
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            let responder = scope.spawn(|| respond(&mut Channel::new(answering), &mut b, "a"));
            let mut other_version = Channel::new(connecting);
            other_version
                .send(&Message::Hello {
                    protocol: PROTOCOL_VERSION + 1,
                    manifest_hash: "whatever".into(),
                })
                .unwrap();
            let answer = other_version.receive(MESSAGE_LIMIT).unwrap();
            assert!(
                matches!(answer, Message::Hello { protocol, .. } if protocol == PROTOCOL_VERSION),
                "the peer learns this machine's version too: {answer:?}"
            );
            assert_eq!(
                responder.join().unwrap().unwrap_err().to_string(),
                "peer a speaks sync protocol 2, this recollect speaks 1; upgrade the older one"
            );
        });
    }

    #[test]
    fn the_initiator_refuses_a_peer_with_another_protocol_version() {
        let mut a = database();
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut other_version = Channel::new(answering);
                other_version.receive(MESSAGE_LIMIT).unwrap();
                other_version
                    .send(&Message::Hello {
                        protocol: PROTOCOL_VERSION + 1,
                        manifest_hash: "whatever".into(),
                    })
                    .unwrap();
            });
            let result = initiate(&mut Channel::new(connecting), &mut a, "b");
            assert_eq!(received_memories(&result), 0);
            assert_eq!(
                result.unwrap_err().to_string(),
                "peer b speaks sync protocol 2, this recollect speaks 1; upgrade the older one"
            );
        });
    }

    #[test]
    fn the_reason_a_peer_gives_is_reported_without_control_characters() {
        let mut a = database();
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut refusing = scripted(answering);
                refusing.receive(MESSAGE_LIMIT).unwrap();
                refusing
                    .send(&Message::Error {
                        message: "no\u{1b}[2J\nc: received 9 memories".into(),
                    })
                    .unwrap();
            });
            let result = initiate(&mut Channel::new(connecting), &mut a, "b");
            assert_eq!(
                result.unwrap_err().to_string(),
                "b reports: no\u{fffd}[2J\u{fffd}c: received 9 memories"
            );
        });
    }

    #[test]
    fn a_message_out_of_turn_ends_the_round() {
        let mut b = database();
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            let responder = scope.spawn(|| respond(&mut Channel::new(answering), &mut b, "a"));
            let mut confused = Channel::new(connecting);
            confused.send(&Message::End {}).unwrap();
            let result = responder.join().unwrap();
            assert_eq!(received_memories(&result), 0);
            assert_eq!(
                result.unwrap_err().to_string(),
                "a sent end where hello was expected"
            );
        });
    }

    #[test]
    fn a_peer_that_hangs_up_mid_round_leaves_the_database_as_it_was() {
        let mut b = database();
        store(&mut b, "from b");
        let (connecting, answering) = socket_pair();
        std::thread::scope(|scope| {
            let responder = scope.spawn(|| respond(&mut Channel::new(answering), &mut b, "a"));
            let mut vanishing = Channel::new(connecting);
            vanishing
                .send(&Message::Hello {
                    protocol: PROTOCOL_VERSION,
                    manifest_hash: "something else".into(),
                })
                .unwrap();
            vanishing.receive(MESSAGE_LIMIT).unwrap();
            vanishing
                .send(&Message::Manifest {
                    entries: Vec::new(),
                })
                .unwrap();
            vanishing.receive(MESSAGE_LIMIT).unwrap();
            drop(vanishing);
            assert_eq!(
                responder.join().unwrap().unwrap_err().to_string(),
                "the peer closed the connection"
            );
        });
        assert_eq!(contents(&b), ["from b"]);
    }

    #[test]
    fn a_responder_that_loses_the_peer_after_storing_reports_what_it_stored() {
        let mut b = database();
        let (entries, memories) = memory_to_send("from the initiator");
        let (connecting, answering) = socket_pair();
        answering
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let result = std::thread::scope(|scope| {
            let responder = scope.spawn(|| respond(&mut Channel::new(answering), &mut b, "a"));
            let mut vanishing = scripted(connecting);
            vanishing
                .send(&Message::Hello {
                    protocol: PROTOCOL_VERSION,
                    manifest_hash: "something else".into(),
                })
                .unwrap();
            vanishing.receive(MESSAGE_LIMIT).unwrap();
            vanishing.send(&Message::Manifest { entries }).unwrap();
            vanishing.receive(MESSAGE_LIMIT).unwrap();
            vanishing.send(&Message::Records { memories }).unwrap();
            vanishing
                .send(&Message::Tombstones {
                    entries: Vec::new(),
                })
                .unwrap();
            vanishing.send(&Message::End {}).unwrap();
            receive_until_end(&mut vanishing);
            drop(vanishing);
            responder.join().unwrap()
        });

        assert_eq!(received_memories(&result), 1);
        assert_eq!(
            result.unwrap_err().to_string(),
            "the peer closed the connection"
        );
        assert_eq!(contents(&b), ["from the initiator"]);
    }

    #[test]
    fn an_initiator_that_loses_the_peer_after_storing_reports_what_it_stored() {
        let mut a = database();
        let (entries, memories) = memory_to_send("from the responder");
        let (connecting, answering) = socket_pair();
        connecting
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut vanishing = scripted(answering);
                vanishing.receive(MESSAGE_LIMIT).unwrap();
                vanishing
                    .send(&Message::Hello {
                        protocol: PROTOCOL_VERSION,
                        manifest_hash: "something else".into(),
                    })
                    .unwrap();
                vanishing.receive(MESSAGE_LIMIT).unwrap();
                vanishing.send(&Message::Manifest { entries }).unwrap();
                receive_until_end(&mut vanishing);
                vanishing.send(&Message::Records { memories }).unwrap();
                vanishing
                    .send(&Message::Tombstones {
                        entries: Vec::new(),
                    })
                    .unwrap();
                vanishing.send(&Message::End {}).unwrap();
                vanishing.receive(MESSAGE_LIMIT).unwrap();
            });
            initiate(&mut Channel::new(connecting), &mut a, "b")
        });

        assert_eq!(received_memories(&result), 1);
        assert_eq!(
            result.unwrap_err().to_string(),
            "the peer closed the connection"
        );
        assert_eq!(contents(&a), ["from the responder"]);
    }
}
