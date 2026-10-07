//! The machines this one syncs with, and the invites it has handed out.

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;

use super::Database;
use crate::error::{Error, Result};

/// A machine this one syncs with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peer {
    /// This machine's label for the peer.
    pub name: String,
    /// The peer's key fingerprint.
    pub fingerprint: String,
    /// Where this machine dials the peer, `host:port`.
    pub address: String,
    pub added_at: String,
    /// When a round with the peer last succeeded.
    pub last_sync_at: Option<String>,
    /// Why a round with the peer last failed. A successful round this
    /// machine started clears it; one the peer started leaves it, because it
    /// does not show that this machine can reach the peer.
    pub last_error: Option<String>,
}

const PEER_COLUMNS: &str = "name, fingerprint, address, added_at, last_sync_at, last_error";

fn peer_from_row(row: &Row<'_>) -> rusqlite::Result<Peer> {
    Ok(Peer {
        name: row.get(0)?,
        fingerprint: row.get(1)?,
        address: row.get(2)?,
        added_at: row.get(3)?,
        last_sync_at: row.get(4)?,
        last_error: row.get(5)?,
    })
}

/// Refuses a key that is already paired and a name that is already taken.
fn check_new_peer(conn: &Connection, name: &str, fingerprint: &str) -> Result<()> {
    let paired_as: Option<String> = conn
        .query_row(
            "SELECT name FROM peers WHERE fingerprint = ?1",
            [fingerprint],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(existing) = paired_as {
        return Err(Error::AlreadyPaired(existing));
    }
    let taken: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM peers WHERE name = ?1)",
        [name],
        |row| row.get(0),
    )?;
    if taken {
        return Err(Error::PeerExists(name.to_string()));
    }
    Ok(())
}

fn insert_peer(
    conn: &Connection,
    name: &str,
    fingerprint: &str,
    address: &str,
    added_at: &str,
) -> Result<()> {
    check_new_peer(conn, name, fingerprint)?;
    conn.execute(
        "INSERT INTO peers (name, fingerprint, address, added_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, fingerprint, address, added_at],
    )?;
    Ok(())
}

impl Database {
    /// Whether a peer with this name and key could be added: the key must not
    /// be paired yet and the name must be free.
    pub fn check_new_peer(&self, name: &str, fingerprint: &str) -> Result<()> {
        check_new_peer(&self.conn, name, fingerprint)
    }

    pub fn add_peer(
        &mut self,
        name: &str,
        fingerprint: &str,
        address: &str,
        added_at: &str,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        insert_peer(&tx, name, fingerprint, address, added_at)?;
        tx.commit()?;
        Ok(())
    }

    /// Every peer, by name.
    pub fn peers(&self) -> Result<Vec<Peer>> {
        let mut statement = self
            .conn
            .prepare(&format!("SELECT {PEER_COLUMNS} FROM peers ORDER BY name"))?;
        let peers = statement.query_map([], peer_from_row)?;
        Ok(peers.collect::<rusqlite::Result<_>>()?)
    }

    pub fn peer_named(&self, name: &str) -> Result<Peer> {
        self.conn
            .query_row(
                &format!("SELECT {PEER_COLUMNS} FROM peers WHERE name = ?1"),
                [name],
                peer_from_row,
            )
            .optional()?
            .ok_or_else(|| Error::UnknownPeer(name.to_string()))
    }

    pub fn peer_with_fingerprint(&self, fingerprint: &str) -> Result<Option<Peer>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {PEER_COLUMNS} FROM peers WHERE fingerprint = ?1"),
                [fingerprint],
                peer_from_row,
            )
            .optional()?)
    }

    /// Changes where this machine dials the peer.
    pub fn set_peer_address(&mut self, name: &str, address: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        let changed = tx.execute(
            "UPDATE peers SET address = ?2 WHERE name = ?1",
            params![name, address],
        )?;
        if changed == 0 {
            return Err(Error::UnknownPeer(name.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_peer(&mut self, name: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        let removed = tx.execute("DELETE FROM peers WHERE name = ?1", [name])?;
        if removed == 0 {
            return Err(Error::UnknownPeer(name.to_string()));
        }
        tx.commit()?;
        Ok(())
    }

    /// Records how a round with the peer `name` went at `at`: a success sets
    /// the time of the last sync and clears the error, a failure stores its
    /// reason. Returns the error stored before, which tells a new failure
    /// from a repeated one. A peer removed in the meantime is ignored.
    pub fn record_round(
        &mut self,
        name: &str,
        at: &str,
        failure: Option<&str>,
    ) -> Result<Option<String>> {
        let tx = self.write_transaction()?;
        let previous: Option<Option<String>> = tx
            .query_row(
                "SELECT last_error FROM peers WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        match failure {
            None => tx.execute(
                "UPDATE peers SET last_sync_at = ?2, last_error = NULL WHERE name = ?1",
                params![name, at],
            )?,
            Some(reason) => tx.execute(
                "UPDATE peers SET last_error = ?2 WHERE name = ?1",
                params![name, reason],
            )?,
        };
        tx.commit()?;
        Ok(previous.flatten())
    }

    /// Records that a round the peer `name` started succeeded at `at`: sets
    /// the time of the last sync. The stored error stays, because it says
    /// why this machine's own last round with the peer failed, and a round
    /// the peer started does not show that the peer can be reached. A peer
    /// removed in the meantime is ignored.
    pub fn record_answered_round(&mut self, name: &str, at: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute(
            "UPDATE peers SET last_sync_at = ?2 WHERE name = ?1",
            params![name, at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Stores the hash of an invite's secret, valid until `expires_at`, and
    /// drops the invites that expired by `now`.
    pub fn add_invite(&mut self, secret_hash: &str, expires_at: &str, now: &str) -> Result<()> {
        let tx = self.write_transaction()?;
        tx.execute("DELETE FROM pairing_invites WHERE expires_at <= ?1", [now])?;
        tx.execute(
            "INSERT INTO pairing_invites (secret_hash, expires_at) VALUES (?1, ?2)",
            params![secret_hash, expires_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Redeems the invite with this secret hash at `now` and stores the
    /// joining machine as a peer, in one transaction. An unknown, expired or
    /// used invite, a taken name or an already paired key changes nothing.
    pub fn redeem_invite(
        &mut self,
        secret_hash: &str,
        now: &str,
        name: &str,
        fingerprint: &str,
        address: &str,
    ) -> Result<()> {
        let tx = self.write_transaction()?;
        let redeemed = tx.execute(
            "UPDATE pairing_invites SET used_at = ?2
             WHERE secret_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![secret_hash, now],
        )?;
        if redeemed == 0 {
            return Err(Error::InvalidInvite);
        }
        insert_peer(&tx, name, fingerprint, address, now)?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{Database, Peer};
    use crate::error::Error;

    const T0: &str = "2026-10-07T10:00:00.000Z";
    const T1: &str = "2026-10-07T10:05:00.000Z";
    const T2: &str = "2026-10-07T10:20:00.000Z";

    fn with_peer(name: &str, fingerprint: &str) -> Database {
        let mut db = Database::open_in_memory().unwrap();
        db.add_peer(name, fingerprint, "host:7327", T0).unwrap();
        db
    }

    fn invite_count(db: &Database) -> i64 {
        db.conn
            .query_row("SELECT count(*) FROM pairing_invites", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn peers_are_listed_by_name_with_what_was_stored() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.add_peer("foehn", "SHA256:f", "foehn:7327", T1).unwrap();
        assert_eq!(
            db.peers().unwrap(),
            [
                Peer {
                    name: "foehn".into(),
                    fingerprint: "SHA256:f".into(),
                    address: "foehn:7327".into(),
                    added_at: T1.into(),
                    last_sync_at: None,
                    last_error: None,
                },
                Peer {
                    name: "twelve".into(),
                    fingerprint: "SHA256:t".into(),
                    address: "host:7327".into(),
                    added_at: T0.into(),
                    last_sync_at: None,
                    last_error: None,
                },
            ]
        );
    }

    #[test]
    fn a_taken_name_and_an_already_paired_key_are_refused() {
        let mut db = with_peer("twelve", "SHA256:t");
        let err = db
            .add_peer("twelve", "SHA256:other", "x:1", T0)
            .unwrap_err();
        assert!(
            matches!(&err, Error::PeerExists(name) if name == "twelve"),
            "{err}"
        );
        assert_eq!(
            err.to_string(),
            "a peer named \"twelve\" already exists; remove it first with: recollect peer remove twelve"
        );
        let err = db.add_peer("laptop", "SHA256:t", "x:1", T0).unwrap_err();
        assert!(
            matches!(&err, Error::AlreadyPaired(name) if name == "twelve"),
            "{err}"
        );
        assert_eq!(
            err.to_string(),
            "this machine is already paired with that key, as peer \"twelve\"; to pair again, remove it first with: recollect peer remove twelve"
        );
        assert!(matches!(
            db.check_new_peer("twelve", "SHA256:new"),
            Err(Error::PeerExists(_))
        ));
        assert!(db.check_new_peer("laptop", "SHA256:new").is_ok());
        assert_eq!(db.peers().unwrap().len(), 1);
    }

    #[test]
    fn peers_are_found_by_name_and_by_fingerprint() {
        let db = with_peer("twelve", "SHA256:t");
        assert_eq!(db.peer_named("twelve").unwrap().fingerprint, "SHA256:t");
        let err = db.peer_named("nobody").unwrap_err();
        assert_eq!(err.to_string(), "no peer named \"nobody\"");
        assert_eq!(
            db.peer_with_fingerprint("SHA256:t").unwrap().unwrap().name,
            "twelve"
        );
        assert_eq!(db.peer_with_fingerprint("SHA256:x").unwrap(), None);
    }

    #[test]
    fn a_peers_address_can_be_changed() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.set_peer_address("twelve", "10.0.0.2:7327").unwrap();
        assert_eq!(db.peer_named("twelve").unwrap().address, "10.0.0.2:7327");
        assert!(matches!(
            db.set_peer_address("nobody", "x:1"),
            Err(Error::UnknownPeer(_))
        ));
    }

    #[test]
    fn a_removed_peer_is_gone_and_removing_it_again_is_an_error() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.remove_peer("twelve").unwrap();
        assert!(db.peers().unwrap().is_empty());
        assert!(matches!(
            db.remove_peer("twelve"),
            Err(Error::UnknownPeer(_))
        ));
    }

    #[test]
    fn a_successful_round_sets_the_sync_time_and_clears_the_error() {
        let mut db = with_peer("twelve", "SHA256:t");
        assert_eq!(
            db.record_round("twelve", T1, Some("unreachable")).unwrap(),
            None
        );
        assert_eq!(
            db.record_round("twelve", T2, None).unwrap().as_deref(),
            Some("unreachable"),
            "the error stored before is returned"
        );
        let peer = db.peer_named("twelve").unwrap();
        assert_eq!(peer.last_sync_at.as_deref(), Some(T2));
        assert_eq!(peer.last_error, None);
    }

    #[test]
    fn a_failed_round_stores_its_reason_and_keeps_the_last_sync_time() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.record_round("twelve", T1, None).unwrap();
        assert_eq!(db.record_round("twelve", T2, Some("first")).unwrap(), None);
        assert_eq!(
            db.record_round("twelve", T2, Some("second"))
                .unwrap()
                .as_deref(),
            Some("first")
        );
        let peer = db.peer_named("twelve").unwrap();
        assert_eq!(peer.last_sync_at.as_deref(), Some(T1));
        assert_eq!(peer.last_error.as_deref(), Some("second"));
    }

    #[test]
    fn a_round_with_a_peer_removed_meanwhile_is_ignored() {
        let mut db = Database::open_in_memory().unwrap();
        assert_eq!(db.record_round("gone", T1, Some("reason")).unwrap(), None);
        assert!(db.peers().unwrap().is_empty());
    }

    #[test]
    fn an_answered_round_sets_the_sync_time_and_keeps_the_error() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.record_round("twelve", T1, Some("unreachable")).unwrap();

        db.record_answered_round("twelve", T2).unwrap();

        let peer = db.peer_named("twelve").unwrap();
        assert_eq!(peer.last_sync_at.as_deref(), Some(T2));
        assert_eq!(peer.last_error.as_deref(), Some("unreachable"));

        db.record_answered_round("gone", T2).unwrap();
        assert_eq!(
            db.peers().unwrap(),
            [peer],
            "a peer removed meanwhile is ignored"
        );
    }

    #[test]
    fn a_valid_invite_makes_the_joining_machine_a_peer_once() {
        let mut db = Database::open_in_memory().unwrap();
        db.add_invite("hash", T2, T0).unwrap();
        db.redeem_invite("hash", T1, "twelve", "SHA256:t", "twelve:7327")
            .unwrap();
        let peer = db.peer_named("twelve").unwrap();
        assert_eq!(
            (
                peer.fingerprint.as_str(),
                peer.address.as_str(),
                peer.added_at.as_str()
            ),
            ("SHA256:t", "twelve:7327", T1)
        );
        let again = db.redeem_invite("hash", T1, "laptop", "SHA256:l", "laptop:7327");
        assert!(matches!(again, Err(Error::InvalidInvite)));
        assert_eq!(db.peers().unwrap().len(), 1);
    }

    #[test]
    fn an_expired_or_unknown_invite_stores_nothing() {
        let mut db = Database::open_in_memory().unwrap();
        db.add_invite("hash", T1, T0).unwrap();
        for (hash, now) in [("hash", T1), ("hash", T2), ("other", T0)] {
            let err = db
                .redeem_invite(hash, now, "twelve", "SHA256:t", "twelve:7327")
                .unwrap_err();
            assert_eq!(
                err.to_string(),
                "this invite is not valid (expired or already used)"
            );
        }
        assert!(db.peers().unwrap().is_empty());
    }

    #[test]
    fn an_invite_survives_a_join_that_is_refused() {
        let mut db = with_peer("twelve", "SHA256:t");
        db.add_invite("hash", T2, T0).unwrap();
        let taken = db.redeem_invite("hash", T1, "twelve", "SHA256:new", "x:1");
        assert!(matches!(taken, Err(Error::PeerExists(_))));
        db.redeem_invite("hash", T1, "laptop", "SHA256:new", "x:1")
            .unwrap();
        assert_eq!(db.peers().unwrap().len(), 2);
    }

    #[test]
    fn adding_an_invite_drops_the_expired_ones() {
        let mut db = Database::open_in_memory().unwrap();
        db.add_invite("old", T1, T0).unwrap();
        db.add_invite("current", T2, T0).unwrap();
        db.add_invite("new", T2, T1).unwrap();
        assert_eq!(
            invite_count(&db),
            2,
            "the invite that expired at T1 is gone"
        );
        assert!(matches!(
            db.redeem_invite("old", T0, "twelve", "SHA256:t", "x:1"),
            Err(Error::InvalidInvite)
        ));
    }
}
