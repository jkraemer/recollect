//! This machine's sync identity: an Ed25519 key in the data directory, the
//! self-signed certificate made from it, and the fingerprint peers know it by.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ParsedCertificate;

use crate::config::Config;
use crate::error::{Error, Result};
use crate::sync::is_valid_peer_name;

/// The key file in the data directory.
pub const IDENTITY_FILE: &str = "identity.key";

/// This machine's key, the certificate it presents, and its fingerprint.
/// Deliberately without `Debug`: it holds the private key.
pub struct Identity {
    certificate: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    fingerprint: String,
}

impl Identity {
    /// Loads the key from the data directory, generating it on first use. An
    /// unreadable or corrupt key is an error and is never replaced: a new key
    /// would break every pairing.
    pub fn load_or_create(data_dir: &Path) -> Result<Self> {
        let path = data_dir.join(IDENTITY_FILE);
        let pem = match std::fs::read_to_string(&path) {
            Ok(pem) => pem,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => create_key(data_dir, &path)?,
            Err(err) => return Err(Error::file(&path)(err)),
        };
        Self::from_pem(&pem).map_err(|reason| Error::Identity {
            path,
            message: format!(
                "not a usable key ({reason}); it is not replaced automatically, because a new key would break every pairing"
            ),
        })
    }

    /// The identity of the PKCS#8 PEM key `pem`; the error says why the key
    /// cannot be used.
    fn from_pem(pem: &str) -> std::result::Result<Self, String> {
        let key_pair = rcgen::KeyPair::from_pem(pem).map_err(|err| match err {
            // The PEM parser's text can quote a line of the file, which is the key.
            rcgen::Error::PemError(_) => "not valid PEM".to_string(),
            err => err.to_string(),
        })?;
        // Peers judge the certificate by its key alone; the name is never checked.
        let certificate = rcgen::CertificateParams::new(vec!["recollect".to_string()])
            .and_then(|params| params.self_signed(&key_pair))
            .map_err(|err| err.to_string())?
            .der()
            .clone();
        let fingerprint = fingerprint_of(&certificate).map_err(|err| err.to_string())?;
        Ok(Self {
            certificate,
            key: PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
            fingerprint,
        })
    }

    /// What peers know this machine by.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// The certificate this machine presents in a TLS handshake.
    pub(crate) fn certificate(&self) -> CertificateDer<'static> {
        self.certificate.clone()
    }

    /// The key that proves the certificate is this machine's.
    pub(crate) fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }
}

/// Generates a key and stores it at `path`, readable by its owner only.
/// Returns the stored key, which is another process's if that one stored its
/// key first.
fn create_key(data_dir: &Path, path: &Path) -> Result<String> {
    let pem = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .map_err(|err| Error::Identity {
            path: path.to_path_buf(),
            message: format!("cannot generate a key: {err}"),
        })?
        .serialize_pem();
    std::fs::create_dir_all(data_dir).map_err(Error::file(data_dir))?;
    let scratch = data_dir.join(format!("{IDENTITY_FILE}.{}", uuid::Uuid::now_v7()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&scratch)
        .map_err(Error::file(&scratch))?;
    // On disk before it is linked: a key file left empty by a power loss
    // would stay, because a key file is never replaced.
    let written = file
        .write_all(pem.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&scratch);
        return Err(Error::file(&scratch)(err));
    }
    // A hard link appears with its content complete, and fails if another
    // process stored its key first.
    let linked = std::fs::hard_link(&scratch, path);
    let _ = std::fs::remove_file(&scratch);
    match linked {
        Ok(()) => Ok(pem),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::read_to_string(path).map_err(Error::file(path))
        }
        Err(err) => Err(Error::file(path)(err)),
    }
}

/// The fingerprint of the key a certificate carries: `SHA256:` and the
/// unpadded URL-safe base64 of the SHA-256 of its SubjectPublicKeyInfo.
pub(crate) fn fingerprint_of(
    certificate: &CertificateDer<'_>,
) -> std::result::Result<String, rustls::Error> {
    let parsed = ParsedCertificate::try_from(certificate)?;
    let digest = ring::digest::digest(
        &ring::digest::SHA256,
        parsed.subject_public_key_info().as_ref(),
    );
    Ok(format!(
        "SHA256:{}",
        URL_SAFE_NO_PAD.encode(digest.as_ref())
    ))
}

/// This machine's name for its peers: `name` from the `[sync]` table of
/// config.toml, else the host name up to its first dot.
pub fn local_name(config: &Config) -> Result<String> {
    if let Some(name) = &config.sync.name {
        return Ok(name.clone());
    }
    let host = host_name();
    let short = host.split('.').next().unwrap_or_default();
    if is_valid_peer_name(short) {
        Ok(short.to_string())
    } else {
        Err(Error::Sync(format!(
            "the host name {host:?} cannot serve as this machine's sync name; set name in the [sync] table of config.toml"
        )))
    }
}

/// Where other machines reach this one unless told otherwise: its host name
/// and the port `recollect serve` listens on.
pub fn default_address(config: &Config) -> String {
    format!("{}:{}", host_name(), config.sync.listen.port())
}

fn host_name() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::*;
    use crate::db::test_support::concurrently;

    fn config(dir: &Path, text: &str) -> Config {
        std::fs::write(dir.join("config.toml"), text).unwrap();
        Config::load_from(dir.to_path_buf(), None).unwrap()
    }

    /// A key that belongs to no machine, made for the test below with
    /// `openssl genpkey -algorithm ed25519`.
    const KNOWN_KEY: &str = "-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIH1ndmtz+ViJT7EDA120UWEmCfxhrtEbxoPxgwMNESjb
-----END PRIVATE KEY-----
";

    /// The fingerprint of `KNOWN_KEY`, computed without recollect: the output
    /// of `openssl pkey -in key.pem -pubout -outform DER | openssl dgst
    /// -sha256 -binary | basenc --base64url` without its padding, with
    /// `SHA256:` in front.
    const KNOWN_KEY_FINGERPRINT: &str = "SHA256:fYzmV92Y_YkgAcdm_pB_uZ7e74SK-hs0Eg-GO4XOZZU";

    /// Peers store fingerprints and invites carry them: a change to how one
    /// is computed would break every pairing made by an earlier release.
    #[test]
    fn the_fingerprint_of_a_known_key_is_the_one_openssl_computes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(IDENTITY_FILE), KNOWN_KEY).unwrap();
        let identity = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(identity.fingerprint(), KNOWN_KEY_FINGERPRINT);
    }

    #[test]
    fn the_first_use_creates_a_key_only_the_owner_can_read() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let identity = Identity::load_or_create(&data_dir).unwrap();
        let fingerprint = identity.fingerprint();
        assert!(fingerprint.starts_with("SHA256:"), "{fingerprint}");
        assert_eq!(fingerprint.len(), "SHA256:".len() + 43, "{fingerprint}");
        let key = data_dir.join(IDENTITY_FILE);
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let stored = rcgen::KeyPair::from_pem(&std::fs::read_to_string(&key).unwrap()).unwrap();
        assert!(stored.is_compatible(&rcgen::PKCS_ED25519), "an Ed25519 key");
        let entries: Vec<_> = std::fs::read_dir(&data_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, [IDENTITY_FILE], "no scratch file is left behind");
    }

    #[test]
    fn the_fingerprint_stays_the_same_across_loads_and_differs_between_machines() {
        let (one, other) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let first = Identity::load_or_create(one.path()).unwrap();
        let again = Identity::load_or_create(one.path()).unwrap();
        assert_eq!(first.fingerprint(), again.fingerprint());
        assert_eq!(
            fingerprint_of(&again.certificate).unwrap(),
            first.fingerprint(),
            "a certificate made anew from the key has the same fingerprint"
        );
        let stranger = Identity::load_or_create(other.path()).unwrap();
        assert_ne!(first.fingerprint(), stranger.fingerprint());
    }

    #[test]
    fn processes_starting_at_once_agree_on_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let fingerprints = concurrently(8, |_| {
            Identity::load_or_create(dir.path())
                .unwrap()
                .fingerprint()
                .to_string()
        });
        assert!(
            fingerprints
                .iter()
                .all(|fingerprint| *fingerprint == fingerprints[0]),
            "{fingerprints:?}"
        );
    }

    #[test]
    fn a_corrupt_key_is_an_error_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join(IDENTITY_FILE);
        std::fs::write(&key, "not a key").unwrap();
        let err = Identity::load_or_create(dir.path()).err().unwrap();
        assert!(
            matches!(&err, Error::Identity { path, message }
                if *path == key && message.starts_with("not a usable key (")),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(&key).unwrap(), "not a key");
    }

    #[test]
    fn a_key_file_the_pem_parser_rejects_is_not_quoted_in_the_error() {
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_create(dir.path()).unwrap();
        let key = dir.path().join(IDENTITY_FILE);
        let pem = std::fs::read_to_string(&key).unwrap();
        let mut lines: Vec<&str> = pem.lines().collect();
        let body: Vec<&str> = lines[1..lines.len() - 1].to_vec();
        assert!(body.len() >= 2, "a blank line goes between two body lines");
        lines.insert(2, "");
        let damaged = lines.join("\n") + "\n";
        std::fs::write(&key, &damaged).unwrap();
        let err = Identity::load_or_create(dir.path()).err().unwrap();
        assert!(
            matches!(&err, Error::Identity { path, message }
                if *path == key && message == "not a usable key (not valid PEM); it is not replaced automatically, because a new key would break every pairing"),
            "{err}"
        );
        for line in body {
            assert!(!err.to_string().contains(line), "{err}");
        }
        assert_eq!(std::fs::read_to_string(&key).unwrap(), damaged);
    }

    #[test]
    fn an_unreadable_key_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join(IDENTITY_FILE);
        std::fs::create_dir(&key).unwrap();
        let err = Identity::load_or_create(dir.path()).err().unwrap();
        assert!(
            matches!(&err, Error::File { path, .. } if *path == key),
            "{err}"
        );
    }

    #[test]
    fn the_name_comes_from_the_configuration_or_the_host_name() {
        let dir = tempfile::tempdir().unwrap();
        let named = config(dir.path(), "[sync]\nname = \"laptop\"\n");
        assert_eq!(local_name(&named).unwrap(), "laptop");
        let unnamed = config(dir.path(), "");
        let host = host_name();
        assert_eq!(
            local_name(&unnamed).unwrap(),
            host.split('.').next().unwrap(),
            "the host name up to its first dot"
        );
    }

    #[test]
    fn the_default_address_is_the_host_name_and_the_listen_port() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(dir.path(), "[sync]\nlisten = \"127.0.0.1:9000\"\n");
        assert_eq!(default_address(&config), format!("{}:9000", host_name()));
    }
}
