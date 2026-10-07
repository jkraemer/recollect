//! Fixtures shared by the sync unit tests: machines with a key and a database
//! of their own, and local sockets.

use std::net::TcpListener;

use crate::config::Config;
use crate::db::Database;
use crate::service::Recollect;
use crate::sync::identity::Identity;
use crate::sync::transport::{self, Connection, Timeouts};

/// When the peers of these tests were added.
pub const T0: &str = "2026-10-07T10:00:00.000Z";

/// What stands where the embedding model's directory would be.
const NO_MODEL: &str = "not-a-directory";

/// A machine with a data directory: its key and its database. Its embedding
/// model can never load, so what a round brings stays without vectors and is
/// warned about.
pub struct Machine {
    pub dir: tempfile::TempDir,
    pub identity: Identity,
}

impl Machine {
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(NO_MODEL), "").unwrap();
        let identity = Identity::load_or_create(dir.path()).unwrap();
        Self { dir, identity }
    }

    /// The machine's configuration as its data directory holds it now.
    pub fn config(&self) -> Config {
        let data_dir = self.dir.path().to_path_buf();
        let no_model = data_dir.join(NO_MODEL);
        Config::load_from(data_dir, Some(no_model)).unwrap()
    }

    /// The machine as the commands open it.
    pub fn app(&self) -> Recollect {
        Recollect::open(self.config()).unwrap()
    }

    /// A connection to the machine's database, as each of its processes
    /// opens one.
    pub fn database(&self) -> Database {
        Database::open(&self.config().database_path()).unwrap()
    }

    /// Makes `other`, dialled at `address`, a peer called `name`.
    pub fn knows(&self, name: &str, other: &Machine, address: &str) {
        self.database()
            .add_peer(name, other.identity.fingerprint(), address, T0)
            .unwrap();
    }

    /// Dials `other` at `address`.
    pub fn connect_to(&self, other: &Machine, address: &str) -> Connection {
        transport::connect(
            &self.identity,
            address,
            other.identity.fingerprint(),
            Timeouts::default(),
        )
        .unwrap()
    }
}

/// A listener on a free local port, and its address.
pub fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    (listener, address)
}

/// A local address nothing listens on.
pub fn closed_address() -> String {
    listener().1
}
