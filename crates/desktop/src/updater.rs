//! The app's own updates, via [`gpui_updater`].
//!
//! The macOS cask declares `auto_updates true`, so updating in place is what
//! brew expects of this app: it reads the installed version back from the
//! bundle rather than from its Caskroom record, and `brew upgrade` only
//! steps in when the bundle falls behind the tap.

use gpui_kit::{App, AppContext as _, Entity, Global};
use gpui_updater::{EngineConfig, StaticManifestSource, Updater, Verification, Version};

const MANIFEST_URL: &str = match option_env!("BYOKEY_DESKTOP_UPDATE_MANIFEST_URL") {
    Some(url) => url,
    None => "https://assets.byokey.io/desktop/stable/latest.json",
};

/// Base64 minisign public key, embedded by the release workflow. A build
/// without one fails closed: every check errors instead of installing an
/// unverified artifact.
const MINISIGN_PUBLIC_KEY: Option<&str> = option_env!("BYOKEY_DESKTOP_UPDATE_MINISIGN_PUBLIC_KEY");

struct SharedUpdater(Entity<Updater>);

impl Global for SharedUpdater {}

/// The running app's version.
#[must_use]
pub fn current_version() -> Version {
    #[expect(
        clippy::expect_used,
        reason = "CARGO_PKG_VERSION is cargo-provided and always valid semver"
    )]
    Version::parse(env!("CARGO_PKG_VERSION")).expect("valid embedded version")
}

/// Publish the shared updater and run one check.
pub fn install(cx: &mut App) {
    let updater = cx.new(|cx| {
        let source = StaticManifestSource::new(MANIFEST_URL)
            .os(std::env::consts::OS)
            .arch(release_arch())
            .format(release_format());
        let mut config = EngineConfig::new(current_version()).verification(Verification::Strict);
        if let Some(key) = MINISIGN_PUBLIC_KEY
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            config = config.minisign_public_key(key);
        }
        Updater::new(source, config, cx)
    });
    updater.update(cx, Updater::check);
    cx.set_global(SharedUpdater(updater));
}

#[must_use]
pub fn shared(cx: &App) -> Entity<Updater> {
    cx.global::<SharedUpdater>().0.clone()
}

fn release_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        arch => arch,
    }
}

fn release_format() -> &'static str {
    match std::env::consts::OS {
        "macos" => "dmg",
        "windows" => "msi",
        _ => "tar.gz",
    }
}
