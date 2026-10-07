//! Sync between machines: paired peers exchange their memories and deletions
//! directly, with no server in between.

pub mod exchange;
pub mod identity;
pub mod protocol;
pub mod transport;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
