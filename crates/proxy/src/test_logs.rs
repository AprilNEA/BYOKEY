//! The log events a test emits, with their fields and the fields of the
//! spans they were logged in, for asserting on what the gateway logs.
//!
//! One subscriber, `tracing-subscriber`'s registry at the server's default
//! `info` level, is installed for the whole test binary and records into a
//! buffer owned by the capturing thread. A per-test thread-local subscriber
//! would race with other tests over `tracing`'s process-wide call-site cache
//! and miss events.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Once;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry::{LookupSpan, Registry};

/// One log event.
#[derive(Debug, Clone)]
pub(crate) struct Logged {
    pub(crate) level: Level,
    pub(crate) message: String,
    pub(crate) fields: BTreeMap<String, String>,
    /// The fields of the spans the event was logged in, the innermost
    /// span's winning a name they share.
    pub(crate) span_fields: BTreeMap<String, String>,
}

impl Logged {
    /// The value of the event's `field`, as it would be printed.
    pub(crate) fn field(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(String::as_str)
    }

    /// The value of `field` on the spans the event was logged in.
    pub(crate) fn span_field(&self, field: &str) -> Option<&str> {
        self.span_fields.get(field).map(String::as_str)
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
            let subscriber = Registry::default().with(LevelFilter::INFO).with(Capture);
            tracing::subscriber::set_global_default(subscriber)
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

/// A span's fields, kept in its registry extensions.
struct SpanFields(BTreeMap<String, String>);

/// Collects values into a message and a field map.
struct Fields<'a> {
    message: Option<&'a mut String>,
    fields: &'a mut BTreeMap<String, String>,
}

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
        match &mut self.message {
            Some(message) if field.name() == "message" => **message = value,
            _ => {
                self.fields.insert(field.name().to_owned(), value);
            }
        }
    }
}

struct Capture;

impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = BTreeMap::new();
        attrs.record(&mut Fields {
            message: None,
            fields: &mut fields,
        });
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(SpanFields(fields));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id)
            && let Some(SpanFields(fields)) = span.extensions_mut().get_mut::<SpanFields>()
        {
            values.record(&mut Fields {
                message: None,
                fields,
            });
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        CAPTURED.with(|c| {
            let mut captured = c.borrow_mut();
            let Some(events) = captured.as_mut() else {
                return;
            };
            let mut message = String::new();
            let mut fields = BTreeMap::new();
            event.record(&mut Fields {
                message: Some(&mut message),
                fields: &mut fields,
            });
            let mut span_fields = BTreeMap::new();
            // From the innermost span outwards.
            for span in ctx.event_scope(event).into_iter().flatten() {
                if let Some(SpanFields(own)) = span.extensions().get::<SpanFields>() {
                    for (name, value) in own {
                        span_fields
                            .entry(name.clone())
                            .or_insert_with(|| value.clone());
                    }
                }
            }
            events.push(Logged {
                level: *event.metadata().level(),
                message,
                fields,
                span_fields,
            });
        });
    }
}
