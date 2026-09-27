use crate::schema::Config;
use arc_swap::ArcSwap;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::watch;

/// Watches a configuration file for changes and hot-reloads on modification.
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
    /// Panics if the OS file watcher cannot be created or the config file path
    /// cannot be registered for watching.
    pub fn watch(self: Arc<Self>) {
        use notify::{RecursiveMode, Watcher as _};
        let watcher_self = Arc::clone(&self);
        let path = self.path.clone();

        tokio::task::spawn_blocking(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher =
                notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if res.is_ok() {
                        let _ = tx.send(());
                    }
                })
                .expect("failed to create watcher");

            watcher
                .watch(&path, RecursiveMode::NonRecursive)
                .expect("failed to watch config file");

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
    fn test_watcher_arc_shared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        write_config(&path, "port: 1111\n");
        let watcher = ConfigWatcher::new(path).unwrap();
        let arc = watcher.arc();
        assert_eq!(arc.load().port, 1111);
    }
}
