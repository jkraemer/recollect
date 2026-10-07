//! Fixtures shared by the sync unit tests: machines with a key and a database
//! of their own, and local sockets.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

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

/// The ports closed addresses are taken from: `CLOSED_PORTS` of them, from
/// `FIRST_CLOSED_PORT` on. They lie far below the ports the system assigns to
/// listeners and to outgoing connections (from 32768 on Linux, from 49152 on
/// macOS), so nothing takes one of them while a test relies on it.
const FIRST_CLOSED_PORT: u16 = 20000;
const CLOSED_PORTS: usize = 10000;

/// How many ports `closed_address` tries before it gives up.
const CANDIDATES: usize = 50;

/// A local address nothing listens on. Each call hands out another port; one
/// that something does listen on is passed over. The end-to-end tests have
/// the same fixture in `tests/sync.rs`.
pub fn closed_address() -> String {
    static HANDED_OUT: AtomicUsize = AtomicUsize::new(0);
    let candidates = std::iter::repeat_with(|| {
        let offset = HANDED_OUT.fetch_add(1, Ordering::Relaxed) % CLOSED_PORTS;
        FIRST_CLOSED_PORT + offset as u16
    });
    first_closed(candidates.take(CANDIDATES)).unwrap_or_else(|| {
        panic!(
            "no closed address: none of {CANDIDATES} local ports tried from {FIRST_CLOSED_PORT} on refused a connection"
        )
    })
}

/// The first of `ports` that refuses a connection, as a local address. A
/// port is tried by dialling it, not by binding it: a listener bound to try
/// it would itself answer a dial for a moment.
fn first_closed(ports: impl IntoIterator<Item = u16>) -> Option<String> {
    ports
        .into_iter()
        .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
        .find(|address| {
            TcpStream::connect_timeout(address, Duration::from_secs(1))
                .is_err_and(|err| err.kind() == ErrorKind::ConnectionRefused)
        })
        .map(|address| address.to_string())
}

mod tests {
    use super::*;

    #[test]
    fn a_closed_address_is_below_the_ports_the_system_assigns_and_refuses_a_dial() {
        let addresses = [closed_address(), closed_address()];
        assert_ne!(addresses[0], addresses[1], "each call hands out another");
        for address in addresses {
            let address: SocketAddr = address.parse().unwrap();
            assert!(address.ip().is_loopback(), "{address}");
            // Linux assigns ports from 32768 on, macOS from higher up.
            assert!(address.port() < 32768, "{address}");
            let refused = TcpStream::connect(address).unwrap_err();
            assert_eq!(refused.kind(), ErrorKind::ConnectionRefused);
        }
    }

    #[test]
    fn a_port_that_answers_a_dial_is_not_a_closed_address() {
        let (listening, _) = listener();
        let answering = listening.local_addr().unwrap().port();
        assert_eq!(first_closed([answering]), None);

        let closed = closed_address();
        let port = closed.parse::<SocketAddr>().unwrap().port();
        assert_eq!(first_closed([answering, port]), Some(closed));
    }
}
