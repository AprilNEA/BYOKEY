use crate::schema::Config;
use arc_swap::ArcSwap;
use std::{path::PathBuf, sync::Arc};

/// Watches a configuration file for changes and hot-reloads on modification.
pub struct ConfigWatcher {
    /// Current configuration, atomically swappable.
    current: Arc<ArcSwap<Config>>,
    /// Path to the configuration file.
    path: PathBuf,
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

    /// Manually reloads the configuration from disk.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] if the configuration file cannot be read or parsed.
    #[allow(clippy::result_large_err)]
    pub fn reload(&self) -> Result<(), figment::Error> {
        let new_config = Config::from_file(&self.path)?;
        self.current.store(Arc::new(new_config));
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
                if let Err(e) = watcher_self.reload() {
                    eprintln!("[byokey-config] reload error: {e}");
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
