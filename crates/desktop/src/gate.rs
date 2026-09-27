//! Whether this app can drive the server it reached.
//!
//! The app is a pure client of a separately installed server, so either side
//! can be newer. The management API's revision decides whether they can talk
//! at all; the release version only decides whether to suggest an update.

use gpui_updater::Version;

/// The oldest management API revision this build still speaks.
pub const MIN_API_VERSION: u32 = 2;
/// The newest management API revision this build knows.
pub const MAX_API_VERSION: u32 = byokey_proto::API_VERSION;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// Compatible and at least as new as this app.
    Ready,
    /// Compatible, but an older release than this app; updating is optional.
    ServerBehind { server: Version },
    /// Too old for this app; nothing works until the server is updated.
    /// A pre-revision server reports `api_version` 0.
    ServerTooOld,
    /// Speaks a revision this app doesn't know yet; the app must update.
    AppTooOld,
}

impl Gate {
    /// Classify a server by the `ServerInfo` it reported.
    #[must_use]
    pub fn classify(api_version: u32, server_version: &str, app: &Version) -> Self {
        if api_version < MIN_API_VERSION {
            return Self::ServerTooOld;
        }
        if api_version > MAX_API_VERSION {
            return Self::AppTooOld;
        }
        match Version::parse(server_version) {
            Ok(server) if server < *app => Self::ServerBehind { server },
            _ => Self::Ready,
        }
    }

    /// Whether the management API is usable under this gate.
    #[must_use]
    pub fn is_usable(&self) -> bool {
        matches!(self, Self::Ready | Self::ServerBehind { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Version {
        Version::new(3, 1, 0)
    }

    #[test]
    fn a_server_without_a_revision_must_update() {
        assert_eq!(Gate::classify(0, "", &app()), Gate::ServerTooOld);
        assert_eq!(
            Gate::classify(MIN_API_VERSION - 1, "3.0.0", &app()),
            Gate::ServerTooOld
        );
    }

    #[test]
    fn a_newer_revision_needs_a_newer_app() {
        assert_eq!(
            Gate::classify(MAX_API_VERSION + 1, "9.0.0", &app()),
            Gate::AppTooOld
        );
    }

    #[test]
    fn an_older_compatible_release_is_a_soft_prompt() {
        assert_eq!(
            Gate::classify(MAX_API_VERSION, "3.0.1", &app()),
            Gate::ServerBehind {
                server: Version::new(3, 0, 1)
            }
        );
    }

    #[test]
    fn an_equal_newer_or_unparsable_release_is_ready() {
        for version in ["3.1.0", "3.2.0", "dev"] {
            assert_eq!(
                Gate::classify(MAX_API_VERSION, version, &app()),
                Gate::Ready,
                "{version}"
            );
        }
    }
}
