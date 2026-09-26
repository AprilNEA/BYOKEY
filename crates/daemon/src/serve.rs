//! The `byokey serve` invocation a background process or OS service runs.

use std::ffi::OsString;
use std::path::PathBuf;

/// The options `byokey serve` is started with. Unset ones fall back to the
/// server's own defaults, except the log file: a detached server has no
/// terminal, so it logs to [`paths::log_path`](crate::paths::log_path)
/// unless told otherwise.
#[derive(Debug, Default, Clone)]
pub struct ServeOptions {
    pub config: Option<PathBuf>,
    pub port: Option<u16>,
    pub host: Option<String>,
    pub db: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
}

impl ServeOptions {
    /// `serve` followed by a flag for each option that is set, except the
    /// log file, which the caller directs.
    #[must_use]
    pub fn args(&self) -> Vec<OsString> {
        let mut args = vec![OsString::from("serve")];
        let mut push = |flag: &str, value: OsString| {
            args.push(flag.into());
            args.push(value);
        };
        if let Some(p) = &self.config {
            push("--config", p.into());
        }
        if let Some(p) = self.port {
            push("--port", p.to_string().into());
        }
        if let Some(h) = &self.host {
            push("--host", h.into());
        }
        if let Some(d) = &self.db {
            push("--db", d.into());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_carry_every_option_given() {
        let opts = ServeOptions {
            config: Some(PathBuf::from("/c.json")),
            port: Some(9),
            host: Some("::1".into()),
            db: Some(PathBuf::from("/t.db")),
            log_file: Some(PathBuf::from("/l.log")),
        };
        let args: Vec<String> = opts
            .args()
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "serve", "--config", "/c.json", "--port", "9", "--host", "::1", "--db", "/t.db"
            ]
        );
        assert_eq!(ServeOptions::default().args(), ["serve"]);
    }
}
