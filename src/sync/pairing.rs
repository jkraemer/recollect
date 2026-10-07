//! Pairing two machines with an invite: one hands out a string naming
//! itself, its address, its key and a one-time secret; the other redeems it.

use std::fmt;
use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use ring::rand::SecureRandom;

use crate::db::{Database, Peer};
use crate::error::{Error, Result};
use crate::sync::identity::Identity;
use crate::sync::protocol::{Channel, MESSAGE_LIMIT, Message, PROTOCOL_VERSION, printable};
use crate::sync::transport::{self, Timeouts};
use crate::sync::{is_valid_address, is_valid_peer_name, sha256_hex};
use crate::time::{format_timestamp, now_timestamp};

/// How long an invite can be redeemed.
pub const INVITE_VALID_MINUTES: i64 = 10;

/// What a machine that is not a peer is told when it sends anything but a
/// valid `pair` message.
pub const NOT_PAIRED: &str =
    "this machine does not know your key; pair again with recollect pair and recollect join";

/// The secret's length before encoding: 128 bits, too many to guess.
const SECRET_BYTES: usize = 16;

/// What one machine hands another so the two can pair: who it is, where to
/// reach it, the key it will prove, and a secret that is good once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    /// The inviting machine's name.
    pub name: String,
    /// Where the joining machine dials it, `host:port`.
    pub address: String,
    /// The inviting machine's key fingerprint.
    pub fingerprint: String,
    /// The one-time secret, unpadded URL-safe base64.
    pub secret: String,
}

impl fmt::Display for Invite {
    /// One token without spaces, easy to select and paste.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{},{},{}",
            self.name, self.address, self.fingerprint, self.secret
        )
    }
}

impl Invite {
    /// Reads an invite as `Display` writes it; surrounding whitespace from a
    /// copy-paste is ignored.
    pub fn parse(text: &str) -> Result<Self> {
        let malformed = |reason: &str| Error::MalformedInvite(reason.to_string());
        let parts: Vec<&str> = text.trim().split(',').collect();
        let [name, address, fingerprint, secret] = parts.as_slice() else {
            return Err(malformed(
                "it must have four parts separated by commas; copy the whole invite",
            ));
        };
        if !is_valid_peer_name(name) {
            return Err(malformed("its first part is not a machine name"));
        }
        if !is_valid_address(address) {
            return Err(malformed("its second part is not a host:port address"));
        }
        let digest = fingerprint.strip_prefix("SHA256:").and_then(decoded);
        if digest.is_none_or(|digest| digest.len() != 32) {
            return Err(malformed("its third part is not a key fingerprint"));
        }
        if decoded(secret).is_none_or(|secret| secret.len() != SECRET_BYTES) {
            return Err(malformed("its last part is not an invite secret"));
        }
        Ok(Self {
            name: name.to_string(),
            address: address.to_string(),
            fingerprint: fingerprint.to_string(),
            secret: secret.to_string(),
        })
    }
}

fn decoded(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

/// Makes an invite for one other machine to pair with this one, which is
/// called `name` and reached at `address`. Only the hash of the secret is
/// stored, valid for `INVITE_VALID_MINUTES` from `now`.
pub fn create_invite(
    db: &mut Database,
    identity: &Identity,
    name: &str,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Invite> {
    let mut secret = [0u8; SECRET_BYTES];
    ring::rand::SystemRandom::new()
        .fill(&mut secret)
        .map_err(|_| Error::Sync("the system gave no random bytes for the invite".to_string()))?;
    let secret = URL_SAFE_NO_PAD.encode(secret);
    db.add_invite(
        &sha256_hex(secret.as_bytes()),
        &format_timestamp(now + TimeDelta::minutes(INVITE_VALID_MINUTES)),
        &format_timestamp(now),
    )?;
    Ok(Invite {
        name: name.to_string(),
        address: address.to_string(),
        fingerprint: identity.fingerprint().to_string(),
        secret,
    })
}

/// The joining side: redeems `invite` with the machine that made it and
/// stores that machine as a peer. This machine introduces itself as
/// `own_name`, reached at `own_address`. The secret is sent only after the
/// inviter has proven the key named in the invite.
pub fn join(
    db: &mut Database,
    identity: &Identity,
    invite: &Invite,
    own_name: &str,
    own_address: &str,
    timeouts: Timeouts,
) -> Result<Peer> {
    if invite.fingerprint == identity.fingerprint() {
        return Err(Error::Sync(
            "this invite was made on this machine; run recollect join on the other one".to_string(),
        ));
    }
    // Checked before anything is sent, so a refusal here leaves the invite unused.
    db.check_new_peer(&invite.name, &invite.fingerprint)?;
    let mut connection =
        transport::connect(identity, &invite.address, &invite.fingerprint, timeouts).map_err(
            |err| match err {
                Error::Unreachable(reason) => Error::Unreachable(format!(
                    "{reason} (is recollect serve running there, and is its port open?)"
                )),
                // Whatever answered is not the inviter, or not a working one:
                // the daemon and the port are not what to check.
                other => other,
            },
        )?;
    // The inviter stores this machine before it answers, so from here on a
    // failure may leave it listed there; the user is told how to repair that.
    let unconfirmed = |reason: String| {
        Error::Sync(format!(
            "{inviter} did not confirm the pairing ({reason}); if recollect peer list on {inviter} shows {own_name}, remove it there with: recollect peer remove {own_name}, then pair again with a new invite",
            inviter = invite.name
        ))
    };
    connection
        .channel
        .send(&Message::Pair {
            protocol: PROTOCOL_VERSION,
            secret: invite.secret.clone(),
            name: own_name.to_string(),
            address: own_address.to_string(),
        })
        .map_err(|err| unconfirmed(err.to_string()))?;
    let answer = connection
        .channel
        .receive(MESSAGE_LIMIT)
        .map_err(|err| unconfirmed(err.to_string()))?;
    match answer {
        Message::Paired {} => {}
        // The inviter stored nothing when it refuses.
        Message::Error { message } => {
            return Err(Error::Sync(format!(
                "{} answered: {}",
                invite.name,
                printable(&message)
            )));
        }
        other => {
            return Err(unconfirmed(format!(
                "it sent {} where paired was expected",
                other.name()
            )));
        }
    }
    db.add_peer(
        &invite.name,
        &invite.fingerprint,
        &invite.address,
        &now_timestamp(),
    )?;
    db.peer_named(&invite.name)
}

/// The inviting side: answers the first message of a machine that is not a
/// peer and has proven the key with `peer_fingerprint`. A valid `pair`
/// message makes it a peer; anything else is refused. The machine is told
/// the outcome. Returns the new peer's name.
pub fn accept<S: Read + Write>(
    channel: &mut Channel<S>,
    db: &mut Database,
    peer_fingerprint: &str,
    message: Message,
    now: DateTime<Utc>,
) -> Result<String> {
    let outcome = redeem(db, peer_fingerprint, message, now);
    let reply = match &outcome {
        Ok(_) => Message::Paired {},
        Err(err) => Message::Error {
            message: err.to_string(),
        },
    };
    // The outcome stands whether or not the machine is still there to hear it.
    let _ = channel.send(&reply);
    outcome
}

fn redeem(
    db: &mut Database,
    peer_fingerprint: &str,
    message: Message,
    now: DateTime<Utc>,
) -> Result<String> {
    let Message::Pair {
        protocol,
        secret,
        name,
        address,
    } = message
    else {
        return Err(Error::Sync(NOT_PAIRED.to_string()));
    };
    if !is_valid_peer_name(&name) {
        return Err(Error::Sync(
            "the name in the pair message is not a machine name".to_string(),
        ));
    }
    if protocol != PROTOCOL_VERSION {
        return Err(Error::ProtocolMismatch {
            peer: name,
            theirs: protocol,
            ours: PROTOCOL_VERSION,
        });
    }
    if !is_valid_address(&address) {
        return Err(Error::Sync(
            "the address in the pair message is not host:port".to_string(),
        ));
    }
    db.redeem_invite(
        &sha256_hex(secret.as_bytes()),
        &format_timestamp(now),
        &name,
        peer_fingerprint,
        &address,
    )?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;
    use crate::sync::protocol::UNPAIRED_MESSAGE_LIMIT;
    use crate::sync::test_support::{Machine, T0, closed_address, listener};

    /// The invite `inviter` hands out at `now`, as `foehn` reached at `address`.
    fn invite_to(inviter: &Machine, address: &str, now: DateTime<Utc>) -> Invite {
        create_invite(
            &mut inviter.database(),
            &inviter.identity,
            "foehn",
            address,
            now,
        )
        .unwrap()
    }

    /// Makes `inviter` listen on a local port and hand out an invite, as
    /// `foehn`, at `now`.
    fn inviting(inviter: &Machine, now: DateTime<Utc>) -> (TcpListener, Invite) {
        let (listener, address) = listener();
        let invite = invite_to(inviter, &address, now);
        (listener, invite)
    }

    /// Answers one connection the way the daemon answers a machine it does
    /// not know, at `now`.
    fn answer_one(listener: &TcpListener, inviter: &Machine, now: DateTime<Utc>) -> Result<String> {
        let (socket, _) = listener.accept().unwrap();
        let mut connection = transport::accept(&inviter.identity, socket, Timeouts::default())?;
        let message = connection.channel.receive(UNPAIRED_MESSAGE_LIMIT)?;
        accept(
            &mut connection.channel,
            &mut inviter.database(),
            &connection.peer_fingerprint,
            message,
            now,
        )
    }

    fn join_as(joiner: &Machine, invite: &Invite, name: &str) -> Result<Peer> {
        join(
            &mut joiner.database(),
            &joiner.identity,
            invite,
            name,
            "twelve.example:7327",
            Timeouts::default(),
        )
    }

    /// Joins while the inviter answers one connection at `answered_at`;
    /// returns what each side reports.
    fn pair(
        inviter: &Machine,
        listener: &TcpListener,
        joiner: &Machine,
        invite: &Invite,
        name: &str,
        answered_at: DateTime<Utc>,
    ) -> (Result<Peer>, Result<String>) {
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| answer_one(listener, inviter, answered_at));
            let joined = join_as(joiner, invite, name);
            (joined, answering.join().unwrap())
        })
    }

    fn message_of<T: std::fmt::Debug>(result: Result<T>) -> String {
        result.unwrap_err().to_string()
    }

    const NOT_VALID: &str = "foehn answered: this invite is not valid (expired or already used)";

    #[test]
    fn an_invite_survives_its_text_form_and_stray_whitespace() {
        let inviter = Machine::new();
        let (_listener, invite) = inviting(&inviter, Utc::now());
        let text = invite.to_string();
        assert_eq!(text.split(',').count(), 4, "{text}");
        assert!(!text.contains(char::is_whitespace), "{text}");
        assert!(text.len() < 130, "about 100 characters: {text}");
        assert_eq!(invite.name, "foehn");
        assert_eq!(invite.fingerprint, inviter.identity.fingerprint());
        assert_eq!(Invite::parse(&text).unwrap(), invite);
        assert_eq!(Invite::parse(&format!("  {text}\n")).unwrap(), invite);
    }

    #[test]
    fn a_mangled_invite_is_refused_with_what_is_wrong() {
        let inviter = Machine::new();
        let (_listener, invite) = inviting(&inviter, Utc::now());
        let with = |change: fn(&mut Invite)| {
            let mut mangled = invite.clone();
            change(&mut mangled);
            mangled.to_string()
        };
        let cases = [
            (
                format!("{},{}", invite.name, invite.address),
                "it must have four parts separated by commas; copy the whole invite",
            ),
            (
                with(|invite| invite.name = "two words".into()),
                "its first part is not a machine name",
            ),
            (
                with(|invite| invite.address = "nowhere".into()),
                "its second part is not a host:port address",
            ),
            (
                with(|invite| invite.fingerprint = "SHA256:short".into()),
                "its third part is not a key fingerprint",
            ),
            (
                with(|invite| invite.secret.truncate(10)),
                "its last part is not an invite secret",
            ),
        ];
        for (text, reason) in cases {
            let err = Invite::parse(&text).unwrap_err();
            assert!(matches!(&err, Error::MalformedInvite(_)), "{err}");
            assert_eq!(err.to_string(), format!("invalid invite: {reason}"));
        }
    }

    #[test]
    fn one_join_leaves_both_machines_knowing_each_other() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let made_at = Utc::now();
        let (listener, invite) = inviting(&inviter, made_at);
        let nine_minutes_later = made_at + TimeDelta::minutes(9);

        let (joined, answered) = pair(
            &inviter,
            &listener,
            &joiner,
            &invite,
            "twelve",
            nine_minutes_later,
        );

        let foehn = joined.unwrap();
        assert_eq!(answered.unwrap(), "twelve");
        assert_eq!(
            (
                foehn.name.as_str(),
                foehn.fingerprint.as_str(),
                foehn.address.as_str()
            ),
            (
                "foehn",
                inviter.identity.fingerprint(),
                invite.address.as_str()
            )
        );
        assert_eq!(joiner.database().peers().unwrap(), [foehn]);
        let twelve = inviter.database().peer_named("twelve").unwrap();
        assert_eq!(
            (twelve.fingerprint.as_str(), twelve.address.as_str()),
            (joiner.identity.fingerprint(), "twelve.example:7327")
        );
        assert_eq!(inviter.database().peers().unwrap().len(), 1);
    }

    #[test]
    fn an_invite_expires_after_ten_minutes_and_pairs_nothing() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let made_at = Utc::now();
        let (listener, invite) = inviting(&inviter, made_at);
        let ten_minutes_later = made_at + TimeDelta::minutes(INVITE_VALID_MINUTES);

        let (joined, answered) = pair(
            &inviter,
            &listener,
            &joiner,
            &invite,
            "twelve",
            ten_minutes_later,
        );

        assert_eq!(message_of(joined), NOT_VALID);
        assert!(matches!(answered, Err(Error::InvalidInvite)));
        assert!(inviter.database().peers().unwrap().is_empty());
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    #[test]
    fn an_invite_works_only_once() {
        let (inviter, first, second) = (Machine::new(), Machine::new(), Machine::new());
        let now = Utc::now();
        let (listener, invite) = inviting(&inviter, now);
        pair(&inviter, &listener, &first, &invite, "twelve", now)
            .0
            .unwrap();

        let (joined, _) = pair(&inviter, &listener, &second, &invite, "laptop", now);

        assert_eq!(message_of(joined), NOT_VALID);
        assert!(second.database().peers().unwrap().is_empty());
        assert_eq!(inviter.database().peers().unwrap().len(), 1);
    }

    #[test]
    fn a_wrong_secret_pairs_nothing() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let now = Utc::now();
        let (listener, invite) = inviting(&inviter, now);
        let (_other_listener, other) = inviting(&Machine::new(), now);
        let guessed = Invite {
            secret: other.secret,
            ..invite
        };

        let (joined, _) = pair(&inviter, &listener, &joiner, &guessed, "twelve", now);

        assert_eq!(message_of(joined), NOT_VALID);
        assert!(inviter.database().peers().unwrap().is_empty());
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_name_taken_on_the_inviting_machine_is_refused_and_stores_nothing() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        inviter
            .database()
            .add_peer("twelve", "SHA256:another-machine", "x:1", T0)
            .unwrap();
        let now = Utc::now();
        let (listener, invite) = inviting(&inviter, now);

        let (joined, _) = pair(&inviter, &listener, &joiner, &invite, "twelve", now);

        assert_eq!(
            message_of(joined),
            "foehn answered: a peer named \"twelve\" already exists; remove it first with: recollect peer remove twelve, or, if it is a different machine, give one of the two another name (name in the [sync] table of config.toml)"
        );
        assert!(joiner.database().peers().unwrap().is_empty());
        assert_eq!(inviter.database().peers().unwrap().len(), 1);

        // The refused join did not use the invite up.
        let (joined, _) = pair(&inviter, &listener, &joiner, &invite, "laptop", now);
        joined.unwrap();
        assert_eq!(inviter.database().peers().unwrap().len(), 2);
    }

    #[test]
    fn a_name_or_key_the_joiner_already_knows_is_refused_before_anything_is_sent() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let now = Utc::now();
        let (listener, invite) = inviting(&inviter, now);

        joiner
            .database()
            .add_peer("foehn", "SHA256:another-machine", "x:1", T0)
            .unwrap();
        // Nobody answers: these joins fail before they connect.
        let taken = join_as(&joiner, &invite, "twelve");
        assert!(matches!(taken, Err(Error::PeerExists(name)) if name == "foehn"));

        joiner.database().remove_peer("foehn").unwrap();
        joiner
            .database()
            .add_peer("desktop", inviter.identity.fingerprint(), "x:1", T0)
            .unwrap();
        let paired = join_as(&joiner, &invite, "twelve");
        assert!(matches!(paired, Err(Error::AlreadyPaired(name)) if name == "desktop"));

        // The invite is still good.
        joiner.database().remove_peer("desktop").unwrap();
        let (joined, _) = pair(&inviter, &listener, &joiner, &invite, "twelve", now);
        joined.unwrap();
    }

    #[test]
    fn a_join_that_cannot_reach_the_inviter_names_the_address_and_what_to_check() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let invite = invite_to(&inviter, &closed_address(), Utc::now());
        let reason = message_of(join_as(&joiner, &invite, "twelve"));
        assert!(
            reason.starts_with(&format!("{}: cannot connect (", invite.address)),
            "{reason}"
        );
        assert!(
            reason.ends_with(" (is recollect serve running there, and is its port open?)"),
            "{reason}"
        );
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_join_that_finds_another_key_at_the_address_says_so_and_sends_nothing() {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let (listener, invite) = inviting(&inviter, Utc::now());
        // Another machine answers where the invite says the inviter is.
        let other = Machine::new();
        let (joined, heard) = std::thread::scope(|scope| {
            let listening = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                transport::accept(&other.identity, socket, Timeouts::default())
                    .and_then(|mut connection| connection.channel.receive(UNPAIRED_MESSAGE_LIMIT))
            });
            let joined = join_as(&joiner, &invite, "twelve");
            (joined, listening.join().unwrap())
        });
        assert_eq!(
            message_of(joined),
            format!(
                "{}: it presented the key {}, not the expected {}",
                invite.address,
                other.identity.fingerprint(),
                invite.fingerprint
            )
        );
        assert!(joiner.database().peers().unwrap().is_empty());
        assert!(
            heard.is_err(),
            "the joiner hung up without sending its pair message"
        );
    }

    #[test]
    fn an_invite_made_on_this_machine_is_refused() {
        let machine = Machine::new();
        let (_listener, invite) = inviting(&machine, Utc::now());
        let err = join_as(&machine, &invite, "foehn");
        assert_eq!(
            message_of(err),
            "this invite was made on this machine; run recollect join on the other one"
        );
    }

    /// Joins as `twelve` while the inviter, played by the test, reads the
    /// `pair` message and then lets `afterwards` act on the connection
    /// instead of answering it; the connection closes when `afterwards` ends.
    fn join_unanswered(afterwards: fn(&mut transport::Connection)) -> (Result<Peer>, Machine) {
        let (inviter, joiner) = (Machine::new(), Machine::new());
        let (listener, invite) = inviting(&inviter, Utc::now());
        let joined = std::thread::scope(|scope| {
            scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                let mut connection =
                    transport::accept(&inviter.identity, socket, Timeouts::default()).unwrap();
                let message = connection.channel.receive(UNPAIRED_MESSAGE_LIMIT).unwrap();
                assert!(matches!(message, Message::Pair { .. }));
                afterwards(&mut connection);
            });
            join_as(&joiner, &invite, "twelve")
        });
        (joined, joiner)
    }

    fn unconfirmed(reason: &str) -> String {
        format!(
            "foehn did not confirm the pairing ({reason}); if recollect peer list on foehn shows twelve, remove it there with: recollect peer remove twelve, then pair again with a new invite"
        )
    }

    #[test]
    fn a_join_whose_answer_never_comes_says_the_inviter_may_have_stored_it() {
        let (joined, joiner) = join_unanswered(|_| {});
        assert_eq!(
            message_of(joined),
            unconfirmed("the peer closed the connection")
        );
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_join_answered_with_another_message_says_the_inviter_may_have_stored_it() {
        let (joined, joiner) = join_unanswered(|connection| {
            connection.channel.send(&Message::End {}).unwrap();
        });
        assert_eq!(
            message_of(joined),
            unconfirmed("it sent end where paired was expected")
        );
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    #[test]
    fn the_reason_an_inviter_gives_is_reported_without_control_characters() {
        let (joined, joiner) = join_unanswered(|connection| {
            connection
                .channel
                .send(&Message::Error {
                    message: "no\u{1b}[2J\npaired with foehn".into(),
                })
                .unwrap();
        });
        assert_eq!(
            message_of(joined),
            "foehn answered: no\u{fffd}[2J\u{fffd}paired with foehn"
        );
        assert!(joiner.database().peers().unwrap().is_empty());
    }

    /// Connects as a machine the inviter does not know, sends `message` and
    /// returns the inviter's reply and what the inviter reports.
    fn send_unpaired(message: Message) -> (Message, Result<String>, Machine) {
        let (inviter, stranger) = (Machine::new(), Machine::new());
        let now = Utc::now();
        let (listener, invite) = inviting(&inviter, now);
        let (reply, answered) = std::thread::scope(|scope| {
            let answering = scope.spawn(|| answer_one(&listener, &inviter, now));
            let mut connection = transport::connect(
                &stranger.identity,
                &invite.address,
                &invite.fingerprint,
                Timeouts::default(),
            )
            .unwrap();
            connection.channel.send(&message).unwrap();
            let reply = connection.channel.receive(MESSAGE_LIMIT).unwrap();
            (reply, answering.join().unwrap())
        });
        (reply, answered, inviter)
    }

    fn pair_message(protocol: u32, name: &str, address: &str) -> Message {
        Message::Pair {
            protocol,
            secret: "whatever".into(),
            name: name.into(),
            address: address.into(),
        }
    }

    #[test]
    fn a_machine_without_an_invite_is_told_it_is_not_paired() {
        let hello = Message::Hello {
            protocol: PROTOCOL_VERSION,
            manifest_hash: "abc".into(),
        };
        let (reply, answered, inviter) = send_unpaired(hello);
        assert_eq!(
            reply,
            Message::Error {
                message: NOT_PAIRED.into()
            }
        );
        assert_eq!(message_of(answered), NOT_PAIRED);
        assert!(inviter.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_pair_message_in_another_protocol_version_is_refused() {
        let (reply, answered, inviter) =
            send_unpaired(pair_message(PROTOCOL_VERSION + 1, "twelve", "twelve:7327"));
        let mismatch =
            "peer twelve speaks sync protocol 2, this recollect speaks 1; upgrade the older one";
        assert_eq!(
            reply,
            Message::Error {
                message: mismatch.into()
            }
        );
        assert_eq!(message_of(answered), mismatch);
        assert!(inviter.database().peers().unwrap().is_empty());
    }

    #[test]
    fn a_pair_message_with_an_unusable_name_or_address_is_refused() {
        for (name, address, reason) in [
            (
                "two words",
                "twelve:7327",
                "the name in the pair message is not a machine name",
            ),
            (
                "twelve",
                "nowhere",
                "the address in the pair message is not host:port",
            ),
        ] {
            let (reply, _, inviter) = send_unpaired(pair_message(PROTOCOL_VERSION, name, address));
            assert_eq!(
                reply,
                Message::Error {
                    message: reason.into()
                }
            );
            assert!(inviter.database().peers().unwrap().is_empty());
        }
    }
}
