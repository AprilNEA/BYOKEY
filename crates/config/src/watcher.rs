use crate::schema::Config;
use arc_swap::ArcSwap;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::watch;

/// Watches a configuration file for changes and hot-reloads on modification.
///
/// The file's directory is watched rather than the file, so the file may be
/// created after the watcher starts, and replaced by a rename (as editors
/// and `byokey route` save it) without the watch going stale: inotify drops
/// a watch on a file that is renamed over. A file that does not exist holds
/// the default configuration.
pub struct ConfigWatcher {
    /// Current configuration, atomically swappable.
    current: Arc<ArcSwap<Config>>,
    /// Path to the configuration file.
    path: PathBuf,
    /// Signalled after every successful reload.
    reloaded: watch::Sender<()>,
}

impl ConfigWatcher {
    /// Creates a new watcher from a file path, loading the initial configuration immediately.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the configuration file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn new(path: PathBuf) -> Result<Self, figment::Error> {
        let config = Config::from_file(&path)?;
        Ok(Self {
            current: Arc::new(ArcSwap::from_pointee(config)),
            path,
            reloaded: watch::Sender::new(()),
        })
    }

    /// Returns a snapshot of the current configuration.
    #[must_use]
    pub fn load(&self) -> arc_swap::Guard<Arc<Config>> {
        self.current.load()
    }

    /// Returns a shareable `ArcSwap` handle (for use in axum `AppState`).
    #[must_use]
    pub fn arc(&self) -> Arc<ArcSwap<Config>> {
        Arc::clone(&self.current)
    }

    /// A receiver that is marked changed after each successful reload, from
    /// a file change or a call to [`reload`](Self::reload), for settings
    /// that need more than reading the new configuration.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<()> {
        self.reloaded.subscribe()
    }

    /// Manually reloads the configuration from disk.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the configuration file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn reload(&self) -> Result<(), figment::Error> {
        let new_config = Config::from_file(&self.path)?;
        self.current.store(Arc::new(new_config));
        self.reloaded.send_replace(());
        Ok(())
    }

    /// Starts background file watching (spawns a tokio task) that automatically
    /// reloads the configuration when the file changes.
    ///
    /// # Panics
    ///
    /// Panics if the OS file watcher cannot be created or the config file's
    /// directory cannot be created or registered for watching.
    pub fn watch(self: Arc<Self>) {
        use notify::{RecursiveMode, Watcher as _};
        let watcher_self = Arc::clone(&self);
        let path = self.path.clone();
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
            _ => PathBuf::from("."),
        };

        tokio::task::spawn_blocking(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher =
                notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if res.is_ok_and(|e| e.paths.iter().any(|p| p.file_name() == path.file_name()))
                    {
                        let _ = tx.send(());
                    }
                })
                .expect("failed to create watcher");

            std::fs::create_dir_all(&dir).expect("failed to create the config directory");
            watcher
                .watch(&dir, RecursiveMode::NonRecursive)
                .expect("failed to watch the config directory");

            for () in rx {
                match watcher_self.reload() {
                    Ok(()) => tracing::info!("configuration reloaded"),
                    Err(e) => tracing::warn!(error = %e, "config reload failed"),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_config(path: &std::path::Path, content: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn test_watcher_initial_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, "port: 9999\n");
        let watcher = ConfigWatcher::new(path).unwrap();
        assert_eq!(watcher.load().port, 9999);
    }

    #[test]
    fn subscribers_hear_of_each_successful_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, "port: 9999\n");
        let watcher = ConfigWatcher::new(path.clone()).unwrap();
        let mut reloads = watcher.subscribe();
        assert!(!reloads.has_changed().unwrap());

        write_config(&path, "port: 8888\n");
        watcher.reload().unwrap();
        assert!(reloads.has_changed().unwrap());
        reloads.mark_unchanged();

        write_config(&path, "port: [not a port\n");
        assert!(watcher.reload().is_err());
        assert!(
            !reloads.has_changed().unwrap(),
            "a failed reload changes nothing"
        );
    }

    #[test]
    fn test_watcher_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, "port: 8317\n");
        let watcher = ConfigWatcher::new(path.clone()).unwrap();
        assert_eq!(watcher.load().port, 8317);

        write_config(&path, "port: 7777\n");
        watcher.reload().unwrap();
        assert_eq!(watcher.load().port, 7777);
    }

    #[test]
    fn invalid_catalog_templates_leave_the_last_valid_configuration_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(
            &path,
            "responses:\n  catalog:\n    name_format: '{{ provider }} / {{ model }}'\n",
        );
        let watcher = ConfigWatcher::new(path.clone()).unwrap();
        let reloads = watcher.subscribe();
        write_config(
            &path,
            "responses:\n  catalog:\n    name_format: '{{ typo }}'\n",
        );

        let error = watcher.reload().unwrap_err();

        assert!(
            error.to_string().contains("responses.catalog.name_format"),
            "{error}"
        );
        assert!(!reloads.has_changed().unwrap());
        assert_eq!(
            watcher.load().responses.catalog.name_formatter().unwrap()("Astra", "Native").unwrap(),
            "Native / Astra"
        );
    }

    #[test]
    fn invalid_anthropic_templates_leave_the_last_valid_configuration_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(
            &path,
            r#"{"anthropic":{"catalog":{"name_format":"{{ provider }} / {{ model }}"}}}"#,
        );
        let watcher = ConfigWatcher::new(path.clone()).unwrap();
        let reloads = watcher.subscribe();
        write_config(
            &path,
            r#"{"anthropic":{"catalog":{"name_format":"{{ typo }}"}}}"#,
        );

        let error = watcher.reload().unwrap_err();

        assert!(
            error.to_string().contains("anthropic.catalog.name_format"),
            "{error}"
        );
        assert!(!reloads.has_changed().unwrap());
        assert_eq!(
            watcher.load().anthropic.catalog.name_formatter().unwrap()("Opus", "Copilot").unwrap(),
            "Copilot / Opus"
        );
    }

    #[test]
    fn obsolete_fields_reject_the_entire_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(
            &path,
            "port: 8123\nanthropic:\n  routes: { default: copilot }\n",
        );
        let watcher = ConfigWatcher::new(path.clone()).unwrap();
        let reloads = watcher.subscribe();
        write_config(&path, "port: 9456\nroutes: { default: cursor }\n");

        let error = watcher.reload().unwrap_err();

        assert!(error.to_string().contains("unknown field"), "{error}");
        assert_eq!(watcher.load().port, 8123);
        assert_eq!(
            watcher.load().anthropic.routes.default.as_deref(),
            Some("copilot")
        );
        assert!(!reloads.has_changed().unwrap());
    }

    /// Wait until `watcher` holds `port`, or fail after a few seconds.
    fn reloaded_to(watcher: &ConfigWatcher, port: u16) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while watcher.load().port != port {
            assert!(
                std::time::Instant::now() < deadline,
                "never reloaded to port {port}"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn a_file_created_or_renamed_over_is_reloaded() {
        // The watch loop never ends, so the runtime must not wait for it.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.json");
        let watcher = Arc::new(ConfigWatcher::new(path.clone()).unwrap());
        assert_eq!(
            watcher.load().port,
            8018,
            "a missing file holds the defaults"
        );
        rt.block_on(async { Arc::clone(&watcher).watch() });
        // The watch registers on a blocking thread.
        std::thread::sleep(std::time::Duration::from_millis(300));

        write_config(&path, r#"{"port": 1111}"#);
        reloaded_to(&watcher, 1111);

        for port in [2222, 3333] {
            let tmp = path.with_extension("tmp");
            write_config(&tmp, &format!(r#"{{"port": {port}}}"#));
            std::fs::rename(&tmp, &path).unwrap();
            reloaded_to(&watcher, port);
        }
        rt.shutdown_background();
    }

    #[test]
    fn test_watcher_arc_shared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, "port: 1111\n");
        let watcher = ConfigWatcher::new(path).unwrap();
        let arc = watcher.arc();
        assert_eq!(arc.load().port, 1111);
    }
}
