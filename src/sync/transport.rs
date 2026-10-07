//! TLS between machines. Both sides present a certificate, and all that
//! identifies a machine is the fingerprint of the key in it: names and
//! validity dates are never checked.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

use rustls::client::Resumption;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, ClientConnection, ConnectionCommon, DigitallySignedStruct, DistinguishedName,
    ServerConfig, ServerConnection, SignatureScheme, StreamOwned,
};

use crate::error::{Error, Result};
use crate::sync::identity::{Identity, fingerprint_of};
use crate::sync::protocol::{Channel, describe_io};

/// How long to wait for another machine.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// For the connection to be made.
    pub connect: Duration,
    /// For any single read or write on it.
    pub io: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            io: Duration::from_secs(30),
        }
    }
}

/// What a channel to another machine runs on.
pub trait Stream: Read + Write + Send {}

impl<T: Read + Write + Send> Stream for T {}

/// An encrypted connection to a machine that has proven it holds a key.
pub struct Connection {
    pub channel: Channel<Box<dyn Stream>>,
    /// The fingerprint of the key the other machine proved it holds.
    pub peer_fingerprint: String,
}

/// Dials `address` and accepts only a machine holding the key with
/// `expected_fingerprint`. An address that does not resolve or that nothing
/// answers at is `Error::Unreachable`; every other failure is `Error::Sync`.
pub fn connect(
    identity: &Identity,
    address: &str,
    expected_fingerprint: &str,
    timeouts: Timeouts,
) -> Result<Connection> {
    let failed = |reason: String| Error::Sync(format!("{address}: {reason}"));
    let mut socket = dial(address, timeouts.connect)
        .map_err(|reason| Error::Unreachable(format!("{address}: {reason}")))?;
    set_timeouts(&socket, timeouts.io).map_err(|err| failed(err.to_string()))?;
    let config =
        client_config(identity, expected_fingerprint).map_err(|err| failed(err.to_string()))?;
    // The name is sent but never checked: the fingerprint decides.
    let name = ServerName::try_from("recollect").expect("a valid DNS name");
    let mut tls =
        ClientConnection::new(Arc::new(config), name).map_err(|err| failed(err.to_string()))?;
    shake_hands(&mut tls, &mut socket).map_err(|err| failed(describe_io(&err)))?;
    Ok(Connection {
        channel: Channel::new(Box::new(StreamOwned::new(tls, socket))),
        peer_fingerprint: expected_fingerprint.to_string(),
    })
}

/// Completes the TLS handshake on a socket another machine opened. Any key
/// may finish it; the caller decides by `peer_fingerprint` what the machine
/// may do.
pub fn accept(
    identity: &Identity,
    mut socket: TcpStream,
    timeouts: Timeouts,
) -> Result<Connection> {
    let failed = |reason: String| Error::Sync(format!("handshake failed: {reason}"));
    set_timeouts(&socket, timeouts.io).map_err(|err| failed(err.to_string()))?;
    let config = server_config(identity).map_err(|err| failed(err.to_string()))?;
    let mut tls = ServerConnection::new(Arc::new(config)).map_err(|err| failed(err.to_string()))?;
    shake_hands(&mut tls, &mut socket).map_err(|err| failed(describe_io(&err)))?;
    let certificate = tls
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or_else(|| failed("the peer presented no certificate".to_string()))?;
    let peer_fingerprint = fingerprint_of(certificate).map_err(|err| failed(err.to_string()))?;
    Ok(Connection {
        channel: Channel::new(Box::new(StreamOwned::new(tls, socket))),
        peer_fingerprint,
    })
}

/// Connects to the first address `address` resolves to that answers; the
/// error says why none did.
fn dial(address: &str, timeout: Duration) -> std::result::Result<TcpStream, String> {
    let candidates = address
        .to_socket_addrs()
        .map_err(|err| format!("cannot resolve the address ({err})"))?;
    let mut last_failure = "the address resolves to nothing".to_string();
    for candidate in candidates {
        match TcpStream::connect_timeout(&candidate, timeout) {
            Ok(socket) => return Ok(socket),
            Err(err) => last_failure = err.to_string(),
        }
    }
    Err(format!("cannot connect ({last_failure})"))
}

fn set_timeouts(socket: &TcpStream, io: Duration) -> std::io::Result<()> {
    socket.set_read_timeout(Some(io))?;
    socket.set_write_timeout(Some(io))
}

/// Runs the handshake to its end, so that its failures surface here and not
/// on the first message.
fn shake_hands<C, D>(tls: &mut C, socket: &mut TcpStream) -> std::io::Result<()>
where
    C: DerefMut<Target = ConnectionCommon<D>>,
{
    while tls.is_handshaking() {
        tls.complete_io(socket)?;
    }
    Ok(())
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

fn client_config(
    identity: &Identity,
    expected_fingerprint: &str,
) -> std::result::Result<ClientConfig, rustls::Error> {
    let provider = provider();
    let verifier = ExpectedKey {
        expected: expected_fingerprint.to_string(),
        algorithms: provider.signature_verification_algorithms,
    };
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(vec![identity.certificate()], identity.private_key())?;
    // No session is kept or resumed: every connection proves its key in a
    // full handshake.
    config.resumption = Resumption::disabled();
    Ok(config)
}

fn server_config(identity: &Identity) -> std::result::Result<ServerConfig, rustls::Error> {
    let provider = provider();
    let verifier = AnyKey {
        algorithms: provider.signature_verification_algorithms,
    };
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_client_cert_verifier(Arc::new(verifier))
        .with_single_cert(vec![identity.certificate()], identity.private_key())?;
    // No ticket to resume a session with is handed out: every connection
    // proves its key in a full handshake.
    config.send_tls13_tickets = 0;
    Ok(config)
}

const NO_TLS_12: &str = "only TLS 1.3 is spoken";

/// The connecting side's check of the other machine's certificate: it must
/// carry exactly the expected key. The handshake's signature check proves
/// that the machine holds that key.
#[derive(Debug)]
struct ExpectedKey {
    expected: String,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for ExpectedKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let presented = fingerprint_of(end_entity)?;
        if presented == self.expected {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "it presented the key {presented}, not the expected {}",
                self.expected
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(NO_TLS_12.to_string()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// The listening side's check of the other machine's certificate: any key
/// may finish the handshake, as long as the certificate can be read and the
/// machine proves it holds the key. Which machine it is, and what it may do,
/// is decided afterwards by the key's fingerprint.
#[derive(Debug)]
struct AnyKey {
    algorithms: WebPkiSupportedAlgorithms,
}

impl ClientCertVerifier for AnyKey {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        fingerprint_of(end_entity)?;
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General(NO_TLS_12.to_string()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use rustls::SupportedProtocolVersion;
    use rustls::client::ResolvesClientCert;
    use rustls::crypto::verify_tls12_signature;
    use rustls::server::{ClientHello, ResolvesServerCert};
    use rustls::sign::CertifiedKey;

    use super::*;
    use crate::sync::protocol::{MESSAGE_LIMIT, Message};

    /// A machine's identity; the directory holds its key and must outlive it.
    fn identity() -> (tempfile::TempDir, Identity) {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::load_or_create(dir.path()).unwrap();
        (dir, identity)
    }

    fn listener() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        (listener, address)
    }

    /// A certificate presented without the key inside it: the signature that
    /// proves possession comes from another machine's key.
    #[derive(Debug)]
    struct StolenCertificate(Arc<CertifiedKey>);

    impl StolenCertificate {
        /// `victim`'s certificate, signed for with `thief`'s key.
        fn of(victim: &Identity, thief: &Identity) -> Arc<Self> {
            let key = provider()
                .key_provider
                .load_private_key(thief.private_key())
                .unwrap();
            Arc::new(Self(Arc::new(CertifiedKey::new(
                vec![victim.certificate()],
                key,
            ))))
        }
    }

    impl ResolvesClientCert for StolenCertificate {
        fn resolve(
            &self,
            _root_hint_subjects: &[&[u8]],
            _sigschemes: &[SignatureScheme],
        ) -> Option<Arc<CertifiedKey>> {
            Some(self.0.clone())
        }

        fn has_certs(&self) -> bool {
            true
        }
    }

    impl ResolvesServerCert for StolenCertificate {
        fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
            Some(self.0.clone())
        }
    }

    /// A hostile client does not care whom it talks to: it accepts any
    /// listener's certificate, in either TLS version.
    #[derive(Debug)]
    struct TrustsAnyListener(WebPkiSupportedAlgorithms);

    impl ServerCertVerifier for TrustsAnyListener {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> std::result::Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(message, cert, dss, &self.0)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(message, cert, dss, &self.0)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.supported_schemes()
        }
    }

    /// A client made from rustls alone, to be given the certificate it
    /// presents (or none) by the test.
    fn hostile_client() -> rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert> {
        hostile_client_offering(&rustls::version::TLS13)
    }

    /// A hostile client that offers only `version`.
    fn hostile_client_offering(
        version: &'static SupportedProtocolVersion,
    ) -> rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert> {
        let provider = provider();
        let algorithms = provider.signature_verification_algorithms;
        ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[version])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(TrustsAnyListener(algorithms)))
    }

    /// Runs a hostile client against `address` until the listener hangs up.
    /// How it ends does not matter: the test judges the listener's side.
    fn run_hostile_client(config: ClientConfig, address: &str) {
        let mut socket = TcpStream::connect(address).unwrap();
        set_timeouts(&socket, Duration::from_secs(5)).unwrap();
        let name = ServerName::try_from("recollect").unwrap();
        let mut tls = ClientConnection::new(Arc::new(config), name).unwrap();
        let _ = shake_hands(&mut tls, &mut socket);
        let _ = rustls::Stream::new(&mut tls, &mut socket).read(&mut [0u8; 16]);
    }

    #[test]
    fn two_machines_prove_their_keys_and_talk_both_ways() {
        let (_a_dir, a) = identity();
        let (_b_dir, b) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                let mut connection = accept(&a, socket, Timeouts::default()).unwrap();
                let received = connection.channel.receive(MESSAGE_LIMIT).unwrap();
                connection.channel.send(&Message::Paired {}).unwrap();
                (connection.peer_fingerprint, received)
            });
            let mut connection =
                connect(&b, &address, a.fingerprint(), Timeouts::default()).unwrap();
            assert_eq!(connection.peer_fingerprint, a.fingerprint());
            connection.channel.send(&Message::End {}).unwrap();
            assert_eq!(
                connection.channel.receive(MESSAGE_LIMIT).unwrap(),
                Message::Paired {}
            );
            let (seen, received) = answering.join().unwrap();
            assert_eq!(
                seen,
                b.fingerprint(),
                "the listening side lets any key in and learns which one it was"
            );
            assert_eq!(received, Message::End {});
        });
    }

    #[test]
    fn the_connecting_side_accepts_only_the_expected_key() {
        let (_a_dir, a) = identity();
        let (_b_dir, b) = identity();
        let (_c_dir, c) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                // The connecting side breaks the handshake off.
                assert!(accept(&a, socket, Timeouts::default()).is_err());
            });
            let err = connect(&b, &address, c.fingerprint(), Timeouts::default())
                .err()
                .unwrap();
            assert_eq!(
                err.to_string(),
                format!(
                    "{address}: it presented the key {}, not the expected {}",
                    a.fingerprint(),
                    c.fingerprint()
                )
            );
        });
    }

    #[test]
    fn a_peer_that_never_answers_is_given_up_on() {
        let (_dir, a) = identity();
        // Connections are queued by the system, but nobody ever answers them.
        let (_listener, address) = listener();
        let timeouts = Timeouts {
            connect: Duration::from_secs(5),
            io: Duration::from_millis(200),
        };
        let err = connect(&a, &address, "SHA256:whoever", timeouts)
            .err()
            .unwrap();
        assert_eq!(
            err.to_string(),
            format!("{address}: the peer did not answer in time")
        );
    }

    #[test]
    fn a_machine_that_is_not_listening_is_reported_with_its_address() {
        let (_dir, a) = identity();
        let (listener, address) = listener();
        drop(listener);
        let err = connect(&a, &address, "SHA256:whoever", Timeouts::default())
            .err()
            .unwrap();
        assert!(matches!(err, Error::Unreachable(_)), "{err}");
        assert!(
            err.to_string()
                .starts_with(&format!("{address}: cannot connect (")),
            "{err}"
        );
    }

    #[test]
    fn an_address_that_cannot_be_resolved_is_reported() {
        let (_dir, a) = identity();
        let address = "no-such-host.invalid:7327";
        let err = connect(&a, address, "SHA256:whoever", Timeouts::default())
            .err()
            .unwrap();
        assert!(matches!(err, Error::Unreachable(_)), "{err}");
        assert!(
            err.to_string()
                .starts_with(&format!("{address}: cannot resolve the address (")),
            "{err}"
        );
    }

    #[test]
    fn something_that_does_not_speak_tls_fails_the_handshake() {
        let (_dir, a) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                accept(&a, socket, Timeouts::default())
                    .err()
                    .map(|err| err.to_string())
            });
            let mut socket = TcpStream::connect(&address).unwrap();
            socket.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
            let err = answering.join().unwrap().expect("the handshake must fail");
            assert!(err.starts_with("handshake failed: "), "{err}");
        });
    }

    #[test]
    fn a_client_presenting_a_certificate_it_holds_no_key_for_is_refused() {
        let (_listening_dir, listening) = identity();
        let (_victim_dir, victim) = identity();
        let (_thief_dir, thief) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                accept(&listening, socket, Timeouts::default())
                    .err()
                    .map(|err| err.to_string())
            });
            let config =
                hostile_client().with_client_cert_resolver(StolenCertificate::of(&victim, &thief));
            run_hostile_client(config, &address);
            assert_eq!(
                answering.join().unwrap().expect("the handshake must fail"),
                "handshake failed: invalid peer certificate: BadSignature"
            );
        });
    }

    #[test]
    fn a_listener_presenting_a_certificate_it_holds_no_key_for_is_refused() {
        let (_dialling_dir, dialling) = identity();
        let (_victim_dir, victim) = identity();
        let (_thief_dir, thief) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let (mut socket, _) = listener.accept().unwrap();
                set_timeouts(&socket, Duration::from_secs(5)).unwrap();
                let config = ServerConfig::builder_with_provider(provider())
                    .with_protocol_versions(&[&rustls::version::TLS13])
                    .unwrap()
                    .with_no_client_auth()
                    .with_cert_resolver(StolenCertificate::of(&victim, &thief));
                let mut tls = ServerConnection::new(Arc::new(config)).unwrap();
                // The dialling machine breaks the handshake off.
                assert!(shake_hands(&mut tls, &mut socket).is_err());
            });
            let err = connect(
                &dialling,
                &address,
                victim.fingerprint(),
                Timeouts::default(),
            )
            .err()
            .unwrap();
            assert_eq!(
                err.to_string(),
                format!("{address}: invalid peer certificate: BadSignature")
            );
        });
    }

    #[test]
    fn a_client_presenting_no_certificate_gets_no_connection() {
        let (_dir, a) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                accept(&a, socket, Timeouts::default())
                    .err()
                    .map(|err| err.to_string())
            });
            run_hostile_client(hostile_client().with_no_client_auth(), &address);
            let err = answering.join().unwrap().expect("the handshake must fail");
            assert!(err.starts_with("handshake failed: "), "{err}");
        });
    }

    /// Why `accept` refuses a client that holds a key and offers only
    /// `version`; `None` if it lets the client in.
    fn refusal_of_a_client_offering(version: &'static SupportedProtocolVersion) -> Option<String> {
        let (_listening_dir, listening) = identity();
        let (_client_dir, client) = identity();
        let (listener, address) = listener();
        std::thread::scope(|scope| {
            let answering = scope.spawn(|| {
                let (socket, _) = listener.accept().unwrap();
                accept(&listening, socket, Timeouts::default())
                    .err()
                    .map(|err| err.to_string())
            });
            let config = hostile_client_offering(version)
                .with_client_auth_cert(vec![client.certificate()], client.private_key())
                .unwrap();
            run_hostile_client(config, &address);
            answering.join().unwrap()
        })
    }

    #[test]
    fn a_client_that_offers_only_tls_1_2_gets_no_connection() {
        assert_eq!(
            refusal_of_a_client_offering(&rustls::version::TLS13),
            None,
            "the same client is let in when it offers TLS 1.3"
        );
        let err =
            refusal_of_a_client_offering(&rustls::version::TLS12).expect("the handshake must fail");
        assert!(err.starts_with("handshake failed: "), "{err}");
    }
}
