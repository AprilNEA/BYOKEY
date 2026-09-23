//! The machine a Copilot account appears to be signed in on.
//!
//! VS Code identifies itself to Copilot with three IDs: a machine id (SHA-256
//! of a MAC address), a device id (a UUID) — both stable for the life of the
//! install — and a session id (`uuid + epoch millis`) minted per window.
//!
//! Each account gets its own stable machine: the ids are derived one-way from
//! the account's credential, so they survive restarts without persisting
//! anything, never repeat across accounts, and reveal nothing about the
//! credential. The session id changes once per process, like a new window.

use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use uuid::Uuid;

/// Random per-process salt, so session ids differ between runs.
static PROCESS_NONCE: LazyLock<[u8; 16]> = LazyLock::new(rand::random);

static PROCESS_START_MS: LazyLock<u128> = LazyLock::new(|| {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
});

#[derive(Clone, Debug)]
pub struct CopilotDevice {
    /// `VScode-MachineId`.
    machine: String,
    /// `Editor-Device-Id`.
    device: String,
    /// `VScode-SessionId`.
    session: String,
}

impl CopilotDevice {
    /// The device for the account holding `credential` — a GitHub OAuth token,
    /// or a Copilot API key when one is configured directly.
    #[must_use]
    pub fn for_credential(credential: &str) -> Self {
        let derive = |purpose: &[u8]| -> [u8; 32] {
            Sha256::new()
                .chain_update(b"byokey/copilot/")
                .chain_update(purpose)
                .chain_update([0])
                .chain_update(credential)
                .finalize()
                .into()
        };
        let session_seed: [u8; 32] = Sha256::new()
            .chain_update(*PROCESS_NONCE)
            .chain_update(derive(b"session"))
            .finalize()
            .into();
        Self {
            machine: hex::encode(derive(b"machine-id")),
            device: uuid_from(&derive(b"dev-device-id")).to_string(),
            session: format!("{}{}", uuid_from(&session_seed), *PROCESS_START_MS),
        }
    }

    pub(super) fn session_id(&self) -> &str {
        &self.session
    }

    /// Headers the Copilot API client attaches to every request.
    #[must_use]
    pub fn headers(&self) -> [(&'static str, &str); 3] {
        [
            ("vscode-sessionid", &self.session),
            ("vscode-machineid", &self.machine),
            ("editor-device-id", &self.device),
        ]
    }
}

/// A v4-shaped UUID taken from a hash, matching `crypto.randomUUID()`.
pub(super) fn uuid_from(hash: &[u8; 32]) -> Uuid {
    uuid::Builder::from_random_bytes(std::array::from_fn(|i| hash[i])).into_uuid()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_credential_is_the_same_machine() {
        let a = CopilotDevice::for_credential("ghu_alpha");
        let b = CopilotDevice::for_credential("ghu_alpha");
        assert_eq!(a.machine, b.machine);
        assert_eq!(a.device, b.device);
        assert_eq!(a.session, b.session);
    }

    #[test]
    fn accounts_do_not_share_a_machine() {
        let a = CopilotDevice::for_credential("ghu_alpha");
        let b = CopilotDevice::for_credential("ghu_beta");
        assert_ne!(a.machine, b.machine);
        assert_ne!(a.device, b.device);
        assert_ne!(a.session, b.session);
    }

    #[test]
    fn ids_have_vscode_shapes() {
        let d = CopilotDevice::for_credential("ghu_alpha");
        assert_eq!(d.machine.len(), 64);
        assert!(d.machine.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(Uuid::parse_str(&d.device).unwrap().get_version_num(), 4);
        let (uuid, millis) = d.session.split_at(36);
        assert!(Uuid::parse_str(uuid).is_ok());
        assert!(millis.parse::<u128>().is_ok());
    }

    #[test]
    fn ids_do_not_contain_the_credential() {
        let d = CopilotDevice::for_credential("ghu_secret_value");
        for (_, v) in d.headers() {
            assert!(!v.contains("secret"));
        }
    }
}
