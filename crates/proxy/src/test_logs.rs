//! The log events a test emits, with their fields, for asserting on what
//! the gateway logs.
//!
//! One subscriber is installed for the whole test binary and records into a
//! buffer owned by the capturing thread. Like the server at its default
//! level, it turns off `debug` and `trace` spans and events. A per-test thread-local subscriber
//! would race with other tests over `tracing`'s process-wide call-site
//! cache and miss events.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Once;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// One log event.
#[derive(Debug, Clone)]
pub(crate) struct Logged {
    pub(crate) level: Level,
    pub(crate) message: String,
    pub(crate) fields: BTreeMap<String, String>,
}

impl Logged {
    /// The value of `field`, as it would be printed.
    pub(crate) fn field(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(String::as_str)
    }
}

thread_local! {
    static CAPTURED: RefCell<Option<Vec<Logged>>> = const { RefCell::new(None) };
}

/// Collects the events of the current thread until dropped. Async tests
/// must run on the current-thread runtime (`#[tokio::test]`'s default).
pub(crate) struct Logs(());

impl Logs {
    pub(crate) fn capture() -> Self {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            tracing::subscriber::set_global_default(Collector::default())
                .expect("no other global subscriber in tests");
        });
        CAPTURED.with(|c| *c.borrow_mut() = Some(Vec::new()));
        Self(())
    }

    /// The events collected so far at `level` or more severe.
    #[allow(
        clippy::unused_self,
        reason = "taking the guard proves capture is on for this thread"
    )]
    pub(crate) fn at_least(&self, level: Level) -> Vec<Logged> {
        CAPTURED.with(|c| {
            c.borrow()
                .iter()
                .flatten()
                .filter(|e| e.level <= level)
                .cloned()
                .collect()
        })
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        CAPTURED.with(|c| *c.borrow_mut() = None);
    }
}

#[derive(Default)]
struct Collector {
    next_span: AtomicU64,
}

struct Fields<'a>(&'a mut Logged);

impl Visit for Fields<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record(field, format!("{value:?}"));
    }
}

impl Fields<'_> {
    fn record(&mut self, field: &Field, value: String) {
        if field.name() == "message" {
            self.0.message = value;
        } else {
            self.0.fields.insert(field.name().to_owned(), value);
        }
    }
}

impl Subscriber for Collector {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::INFO
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        CAPTURED.with(|c| {
            if let Some(events) = c.borrow_mut().as_mut() {
                let mut logged = Logged {
                    level: *event.metadata().level(),
                    message: String::new(),
                    fields: BTreeMap::new(),
                };
                event.record(&mut Fields(&mut logged));
                events.push(logged);
            }
        });
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}
