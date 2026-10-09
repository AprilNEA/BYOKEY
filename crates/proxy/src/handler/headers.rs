//! Configured upstream headers shared by Messages and Responses.

use byokey_config::ConfigValue;
use byokey_types::ByokError;
use http::{HeaderMap, HeaderName, HeaderValue};
use std::collections::BTreeMap;

pub(super) fn apply(
    headers: &mut HeaderMap,
    configured: &BTreeMap<String, ConfigValue>,
) -> Result<(), ByokError> {
    for (name, source) in configured {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ByokError::Config(format!("invalid upstream header name: {name}")))?;
        let mut value = HeaderValue::from_str(&source.resolve()?)
            .map_err(|_| ByokError::Config(format!("invalid value for upstream header {name}")))?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    strip_hop_headers(headers);
    Ok(())
}

/// Remove connection-specific headers, including the fields named by Connection.
pub(super) fn strip_hop_headers(headers: &mut HeaderMap) {
    let named: Vec<_> = headers
        .get_all("connection")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(str::trim))
        .map(str::to_owned)
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "content-length",
    ] {
        headers.remove(name);
    }
}
