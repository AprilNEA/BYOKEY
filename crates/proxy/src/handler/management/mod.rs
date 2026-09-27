//! BYOKEY management `ConnectRPC` services, used by `byokey tui` and the
//! desktop app.
//!
//! - `StatusService` — server identity, provider login state, usage
//! - `AccountsService` — stored accounts, and adding, switching and removing them

// The generated traits return `impl Future`, so a handler that has nothing
// to await still declares `async fn` to satisfy them.
#![allow(
    clippy::unused_async_trait_impl,
    reason = "handler signatures follow the generated trait"
)]
// The traits declare `ServiceResult<impl Encodable<M>>` so a handler may
// return either the owned message or a borrowed view. Returning the owned
// type refines that bound, which connectrpc's own guide calls the intended
// use and tells callers to allow.
#![allow(
    refining_impl_trait,
    reason = "handlers return the owned response type, as connectrpc's guide prescribes"
)]

mod accounts;
mod status;

use std::sync::Arc;

use byokey_types::{ByokError, ProviderId};
use connectrpc::{ConnectError, Router as ConnectRouter};

use byokey_proto::byokey::accounts as acct;
use byokey_proto::byokey::status as stat;

use crate::AppState;
use accounts::AccountsServiceImpl;
use status::StatusServiceImpl;

/// Build a [`ConnectRouter`] with all management services registered.
#[must_use]
pub fn build_router(state: Arc<AppState>) -> ConnectRouter {
    use acct::AccountsServiceExt as _;
    use stat::StatusServiceExt as _;

    let router = ConnectRouter::new();
    let router = Arc::new(StatusServiceImpl(state.clone())).register(router);
    Arc::new(AccountsServiceImpl(state)).register(router)
}

fn parse_provider(s: &str) -> Result<ProviderId, ConnectError> {
    s.parse()
        .map_err(|e: ByokError| ConnectError::invalid_argument(e.to_string()))
}

/// Map a store/auth failure onto a Connect status the client can act on.
fn to_connect_error(e: &ByokError) -> ConnectError {
    match e {
        ByokError::Auth(_) | ByokError::UnsupportedProvider(_) => {
            ConnectError::invalid_argument(e.to_string())
        }
        ByokError::TokenNotFound(_) | ByokError::AccountNotFound { .. } => {
            ConnectError::not_found(e.to_string())
        }
        _ => ConnectError::internal(e.to_string()),
    }
}
