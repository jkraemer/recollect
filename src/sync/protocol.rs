//! The sync wire format: JSON messages, each framed by a 4-byte big-endian
//! length.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::memory::{SyncRecord, Tombstone};

/// The version of what travels between machines and how it is read. Bump it
/// whenever messages, their order, the fields of a record or the values a
/// field may take change; peers with different versions refuse each other.
/// Releases and schema migrations that leave the wire alone do not touch it.
/// The `hello` message must keep its shape in every version: it is how
/// peers find out that they differ.
pub const PROTOCOL_VERSION: u32 = 1;

/// The largest message a peer may send.
pub const MESSAGE_LIMIT: usize = 16 * 1024 * 1024;

/// The largest message accepted from a machine that is not a peer.
pub const UNPAIRED_MESSAGE_LIMIT: usize = 4 * 1024;

/// Whether a memory in a manifest is live or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryState {
    Live,
    Deleted,
}

/// One memory in a manifest: its global id and its state.
pub type ManifestEntry = (String, EntryState);

/// Everything two machines say to each other. `End` and `Paired` are empty
/// struct variants because serde ignores unknown fields on an internally
/// tagged unit variant, and an unknown field must be refused in every message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    /// Opens a round: the sender's protocol version and the hash of its manifest.
    Hello {
        protocol: u32,
        manifest_hash: String,
    },
    /// Every memory the sender holds, with its state.
    Manifest { entries: Vec<ManifestEntry> },
    /// Memories the receiver has never seen, live or deleted.
    Records { memories: Vec<SyncRecord> },
    /// Deletions of memories the receiver still holds live.
    Tombstones { entries: Vec<Tombstone> },
    /// The sender has sent all its changes.
    End {},
    /// What the sender applied of the receiver's changes.
    Applied { inserted: usize, deleted: usize },
    /// A machine that is not a peer yet redeems an invite.
    Pair {
        protocol: u32,
        secret: String,
        name: String,
        address: String,
    },
    /// The invite was accepted.
    Paired {},
    /// The sender gives up on the round or the pairing, and says why.
    Error { message: String },
}

impl Message {
    /// The message's `type`, for error texts.
    pub fn name(&self) -> &'static str {
        match self {
            Message::Hello { .. } => "hello",
            Message::Manifest { .. } => "manifest",
            Message::Records { .. } => "records",
            Message::Tombstones { .. } => "tombstones",
            Message::End {} => "end",
            Message::Applied { .. } => "applied",
            Message::Pair { .. } => "pair",
            Message::Paired {} => "paired",
            Message::Error { .. } => "error",
        }
    }
}

/// Messages over a byte stream, each framed by its length.
pub struct Channel<S> {
    stream: S,
}

impl<S: Read + Write> Channel<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    pub fn send(&mut self, message: &Message) -> Result<()> {
        let json = serde_json::to_vec(message).expect("messages always serialize");
        if json.len() > MESSAGE_LIMIT {
            return Err(Error::Sync(format!(
                "{} {} message of {} bytes exceeds the limit of {MESSAGE_LIMIT}",
                if message.name().starts_with(['a', 'e']) {
                    "an"
                } else {
                    "a"
                },
                message.name(),
                json.len()
            )));
        }
        // The limit keeps the length within 32 bits.
        let length = (json.len() as u32).to_be_bytes();
        self.stream
            .write_all(&length)
            .and_then(|()| self.stream.write_all(&json))
            .and_then(|()| self.stream.flush())
            .map_err(|err| Error::Sync(describe_io(&err)))
    }

    /// Reads the next message; one longer than `limit` bytes is refused
    /// before its body is read.
    pub fn receive(&mut self, limit: usize) -> Result<Message> {
        let failed = |err: std::io::Error| Error::Sync(describe_io(&err));
        let mut length = [0u8; 4];
        self.stream.read_exact(&mut length).map_err(failed)?;
        let length = u32::from_be_bytes(length) as usize;
        if length > limit {
            return Err(Error::Sync(format!(
                "the peer sent a message of {length} bytes; the limit is {limit}"
            )));
        }
        let mut json = vec![0u8; length];
        self.stream.read_exact(&mut json).map_err(failed)?;
        serde_json::from_slice(&json).map_err(|err| {
            Error::Sync(format!(
                "the peer sent a message this recollect cannot read: {err}"
            ))
        })
    }
}

/// Says in words why reading from or writing to a peer failed.
pub(crate) fn describe_io(err: &std::io::Error) -> String {
    use std::io::ErrorKind;
    let tls = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>());
    match (tls, err.kind()) {
        (Some(rustls::Error::General(reason)), _) => reason.clone(),
        (Some(other), _) => other.to_string(),
        (None, ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::BrokenPipe) => {
            "the peer closed the connection".to_string()
        }
        (None, ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            "the peer did not answer in time".to_string()
        }
        (None, _) => err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::memory::MemoryType;

    /// The JSON in `fixtures` describes protocol 1. A change that alters it
    /// changes what travels between machines: bump `PROTOCOL_VERSION` and
    /// update the fixtures together, on purpose.
    #[test]
    fn the_fixtures_describe_the_current_protocol() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    fn fixtures() -> Vec<(Message, &'static str)> {
        vec![
            (
                Message::Hello {
                    protocol: 1,
                    manifest_hash: "abc".into(),
                },
                r#"{"type":"hello","protocol":1,"manifest_hash":"abc"}"#,
            ),
            (
                Message::Manifest {
                    entries: vec![
                        (
                            "0199a8c0-0000-7000-8000-000000000001".into(),
                            EntryState::Live,
                        ),
                        (
                            "0199a8c0-0000-7000-8000-000000000002".into(),
                            EntryState::Deleted,
                        ),
                    ],
                },
                r#"{"type":"manifest","entries":[["0199a8c0-0000-7000-8000-000000000001","live"],["0199a8c0-0000-7000-8000-000000000002","deleted"]]}"#,
            ),
            (
                Message::Records {
                    memories: vec![SyncRecord {
                        global_id: "0199a8c0-0000-7000-8000-000000000001".into(),
                        project: Some("recollect".into()),
                        memory_type: MemoryType::Todo,
                        content: "Ship sync".into(),
                        tags: vec!["sync".into()],
                        origin_peer: None,
                        created_at: "2026-10-07T10:00:00.000Z".into(),
                        deleted_at: None,
                        deleted_by_peer: None,
                    }],
                },
                r#"{"type":"records","memories":[{"global_id":"0199a8c0-0000-7000-8000-000000000001","project":"recollect","memory_type":"todo","content":"Ship sync","tags":["sync"],"origin_peer":null,"created_at":"2026-10-07T10:00:00.000Z","deleted_at":null,"deleted_by_peer":null}]}"#,
            ),
            (
                Message::Tombstones {
                    entries: vec![Tombstone {
                        global_id: "0199a8c0-0000-7000-8000-000000000002".into(),
                        deleted_at: "2026-10-07T11:00:00.000Z".into(),
                        deleted_by_peer: None,
                    }],
                },
                r#"{"type":"tombstones","entries":[{"global_id":"0199a8c0-0000-7000-8000-000000000002","deleted_at":"2026-10-07T11:00:00.000Z","deleted_by_peer":null}]}"#,
            ),
            (Message::End {}, r#"{"type":"end"}"#),
            (
                Message::Applied {
                    inserted: 3,
                    deleted: 1,
                },
                r#"{"type":"applied","inserted":3,"deleted":1}"#,
            ),
            (
                Message::Pair {
                    protocol: 1,
                    secret: "c2VjcmV0".into(),
                    name: "twelve".into(),
                    address: "twelve:7327".into(),
                },
                r#"{"type":"pair","protocol":1,"secret":"c2VjcmV0","name":"twelve","address":"twelve:7327"}"#,
            ),
            (Message::Paired {}, r#"{"type":"paired"}"#),
            (
                Message::Error {
                    message: "this invite is not valid (expired or already used)".into(),
                },
                r#"{"type":"error","message":"this invite is not valid (expired or already used)"}"#,
            ),
        ]
    }

    #[test]
    fn every_message_has_exactly_this_json() {
        for (message, json) in fixtures() {
            assert_eq!(serde_json::to_string(&message).unwrap(), json);
            assert_eq!(serde_json::from_str::<Message>(json).unwrap(), message);
        }
    }

    #[test]
    fn every_message_knows_its_type_name() {
        for (message, json) in fixtures() {
            assert!(
                json.starts_with(&format!(r#"{{"type":"{}""#, message.name())),
                "{json}"
            );
        }
    }

    #[test]
    fn unknown_fields_messages_and_memory_types_are_refused() {
        for json in [
            r#"{"type":"hello","protocol":1,"manifest_hash":"abc","extra":true}"#,
            r#"{"type":"end","extra":1}"#,
            r#"{"type":"paired","extra":1}"#,
            r#"{"type":"records","memories":[{"global_id":"g","project":null,"memory_type":"note","content":"c","tags":[],"origin_peer":null,"created_at":"t","deleted_at":null,"deleted_by_peer":null,"extra":1}]}"#,
            r#"{"type":"greeting"}"#,
            r#"{"protocol":1,"manifest_hash":"abc"}"#,
            r#"{"type":"tombstones","entries":[{"global_id":"g","deleted_at":"t","deleted_by_peer":null,"why":"x"}]}"#,
            r#"{"type":"records","memories":[{"global_id":"g","project":null,"memory_type":"decision","content":"c","tags":[],"origin_peer":null,"created_at":"t","deleted_at":null,"deleted_by_peer":null}]}"#,
        ] {
            assert!(serde_json::from_str::<Message>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn messages_survive_the_trip_through_a_channel() {
        let mut channel = Channel::new(Cursor::new(Vec::new()));
        for (message, _) in fixtures() {
            channel.send(&message).unwrap();
        }
        channel.stream.set_position(0);
        for (message, _) in fixtures() {
            assert_eq!(channel.receive(MESSAGE_LIMIT).unwrap(), message);
        }
    }

    #[test]
    fn a_frame_is_a_big_endian_length_and_the_json() {
        let mut channel = Channel::new(Cursor::new(Vec::new()));
        channel.send(&Message::End {}).unwrap();
        let mut expected = vec![0, 0, 0, 14];
        expected.extend_from_slice(br#"{"type":"end"}"#);
        assert_eq!(channel.stream.into_inner(), expected);
    }

    #[test]
    fn a_message_over_the_limit_is_refused_before_it_is_read() {
        let mut channel = Channel::new(Cursor::new(Vec::new()));
        channel.send(&Message::End {}).unwrap();
        channel.stream.set_position(0);
        let err = channel.receive(10).unwrap_err();
        assert_eq!(
            err.to_string(),
            "the peer sent a message of 14 bytes; the limit is 10"
        );
    }

    #[test]
    fn a_message_too_large_to_send_is_refused() {
        let mut channel = Channel::new(Cursor::new(Vec::new()));
        let huge = Message::Error {
            message: "x".repeat(MESSAGE_LIMIT),
        };
        let err = channel.send(&huge).unwrap_err();
        assert!(err.to_string().starts_with("an error message of "), "{err}");
        assert!(
            channel.stream.into_inner().is_empty(),
            "nothing was written"
        );
    }

    #[test]
    fn a_connection_closed_mid_message_is_reported() {
        let mut bytes = vec![0, 0, 0, 14];
        bytes.extend_from_slice(b"{\"ty");
        let mut channel = Channel::new(Cursor::new(bytes));
        let err = channel.receive(MESSAGE_LIMIT).unwrap_err();
        assert_eq!(err.to_string(), "the peer closed the connection");
    }

    #[test]
    fn unreadable_json_is_reported() {
        let mut bytes = vec![0, 0, 0, 2];
        bytes.extend_from_slice(b"{]");
        let mut channel = Channel::new(Cursor::new(bytes));
        let err = channel.receive(MESSAGE_LIMIT).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("the peer sent a message this recollect cannot read: "),
            "{err}"
        );
    }

    #[test]
    fn i_o_failures_are_put_in_words() {
        use std::io::{Error as IoError, ErrorKind};
        for (err, expected) in [
            (
                IoError::from(ErrorKind::UnexpectedEof),
                "the peer closed the connection",
            ),
            (
                IoError::from(ErrorKind::ConnectionReset),
                "the peer closed the connection",
            ),
            (
                IoError::from(ErrorKind::TimedOut),
                "the peer did not answer in time",
            ),
            (
                IoError::from(ErrorKind::WouldBlock),
                "the peer did not answer in time",
            ),
            (
                IoError::new(
                    ErrorKind::InvalidData,
                    rustls::Error::General("it presented another key".into()),
                ),
                "it presented another key",
            ),
        ] {
            assert_eq!(describe_io(&err), expected);
        }
    }
}
