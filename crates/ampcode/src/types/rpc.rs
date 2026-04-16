//! JSON-RPC wire format types for `POST /api/internal`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The JSON body sent to `POST /api/internal`.
#[derive(Debug, Serialize)]
pub struct RpcRequest<'a> {
    /// JSON-RPC method name (e.g. `"userDisplayBalanceInfo"`).
    pub method: &'a str,
    /// Method parameters. Defaults to `{}` when `None`.
    #[serde(serialize_with = "serialize_params")]
    pub params: Option<Value>,
}

/// Serialize `params` as `{}` when `None`, since the Ampcode API
/// requires `params` to be a present object.
#[allow(clippy::ref_option)] // serde's serialize_with requires &Option<T>
fn serialize_params<S: serde::Serializer>(
    val: &Option<Value>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    match val {
        Some(v) => v.serialize(s),
        None => serde_json::Value::Object(serde_json::Map::new()).serialize(s),
    }
}

/// Wire envelope for JSON-RPC responses: `{"ok": true, "result": {...}}`.
#[derive(Debug, Deserialize)]
pub struct RpcResponseEnvelope<T> {
    /// Inner result payload.
    pub result: T,
}

/// Wire type for the `userDisplayBalanceInfo` RPC response
/// (the inner `result` object).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceInfoRaw {
    /// Formatted balance string shown in the Ampcode UI.
    pub display_text: String,
}
