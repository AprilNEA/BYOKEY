//! Cursor login: the Cursor CLI's PKCE browser flow, and API key exchange.
//!
//! Both end in a short-lived access token (a JWT). A browser login stores the
//! access token with its refresh JWT; an API key (`crsr_…` from
//! cursor.com/dashboard, in config or stored with `byokey add-api-key cursor`)
//! is itself the long-lived credential and is exchanged per use by the
//! executor. Either credential is traded for a fresh access token at the same
//! exchange endpoint.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use byokey_types::{ByokError, OAuthToken, Result};
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::AuthManager;
use crate::flow::{LoginProgress, emit, open_browser, save_login_token};
use crate::pkce;

const WEBSITE: &str = "https://cursor.com";
const API: &str = "https://api2.cursor.sh";
/// How long the browser half of the login may take.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenPair {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// The browser URL that starts a login for `uuid` with the given PKCE challenge.
fn login_url(challenge: &str, uuid: &str) -> String {
    format!(
        "{WEBSITE}/loginDeepControl?challenge={challenge}&uuid={uuid}&mode=login&redirectTarget=cli"
    )
}

/// Unix expiry of a JWT, read without verifying it.
fn jwt_expiry(jwt: &str) -> Result<u64> {
    let claims = jwt
        .split('.')
        .nth(1)
        .and_then(|b| URL_SAFE_NO_PAD.decode(b.trim_end_matches('=')).ok())
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .ok_or_else(|| ByokError::Auth("Cursor access token is not a JWT".into()))?;
    claims
        .get("exp")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ByokError::Auth("Cursor access token has no expiry".into()))
}

/// Refresh credentials that are JWTs are rotated by the server; anything
/// else (an API key) stays as the long-lived credential.
fn is_jwt(s: &str) -> bool {
    s.split('.').count() == 3
}

fn token_from(pair: TokenPair, credential: Option<&str>) -> Result<OAuthToken> {
    let expires_at = jwt_expiry(&pair.access_token)?;
    let refresh = pair
        .refresh_token
        .filter(|r| is_jwt(r))
        .or_else(|| credential.map(str::to_owned));
    let mut token = OAuthToken::new(pair.access_token);
    token.expires_at = Some(expires_at);
    token.refresh_token = refresh;
    Ok(token)
}

/// Trade a refresh credential (refresh JWT or `crsr_` API key) for a fresh
/// access token.
///
/// # Errors
///
/// Returns [`ByokError::Auth`] with an `invalid_grant:` prefix when Cursor
/// rejects the credential, or a network/parse error.
pub async fn exchange(http: &wreq::Client, credential: &str) -> Result<OAuthToken> {
    let resp = http
        .post(format!("{API}/auth/exchange_user_api_key"))
        .bearer_auth(credential)
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await?;
    let status = resp.status();
    if status.is_client_error() && status.as_u16() != 408 && status.as_u16() != 429 {
        return Err(ByokError::Auth(format!(
            "invalid_grant: Cursor rejected the credential (HTTP {status})"
        )));
    }
    if !status.is_success() {
        return Err(ByokError::Auth(format!(
            "Cursor token exchange failed (HTTP {status})"
        )));
    }
    token_from(resp.json::<TokenPair>().await?, Some(credential))
}

/// Run the Cursor CLI browser login: open `loginDeepControl`, then poll until
/// the user approves.
///
/// # Errors
///
/// Returns an error if the user rejects the login, it times out, or the
/// token cannot be saved.
pub async fn login(
    auth: &AuthManager,
    http: &wreq::Client,
    account: Option<&str>,
    events: Option<&mpsc::Sender<LoginProgress>>,
) -> Result<()> {
    emit(events, LoginProgress::Started).await;
    let (verifier, challenge) = pkce::generate_pkce();
    let uuid = uuid::Uuid::new_v4().to_string();
    let url = login_url(&challenge, &uuid);
    if events.is_none() {
        println!("Open this URL in your browser: {url}");
    }
    open_browser(&url);
    emit(
        events,
        LoginProgress::OpenedBrowser {
            url,
            user_code: None,
        },
    )
    .await;

    let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
    let pair = loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("Cursor login timed out".into()));
        }
        let resp = http
            .get(format!("{API}/auth/poll"))
            .query(&[("uuid", uuid.as_str()), ("verifier", verifier.as_str())])
            .send()
            .await?;
        match resp.status().as_u16() {
            // Not approved yet.
            404 => {}
            200 => break resp.json::<TokenPair>().await?,
            403 => return Err(ByokError::Auth("Cursor rejected the login".into())),
            s => {
                return Err(ByokError::Auth(format!(
                    "Cursor login poll failed (HTTP {s})"
                )));
            }
        }
    };
    emit(events, LoginProgress::Exchanging).await;
    let token = token_from(pair, None)?;
    save_login_token(auth, &byokey_types::ProviderId::Cursor, token, account).await?;
    if events.is_none() {
        println!("cursor login successful");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(exp: u64) -> String {
        let claims = URL_SAFE_NO_PAD.encode(format!(r#"{{"exp":{exp}}}"#));
        format!("h.{claims}.s")
    }

    #[test]
    fn api_key_is_kept_as_refresh_credential() {
        let pair = TokenPair {
            access_token: jwt(100),
            refresh_token: Some("crsr_echo".into()),
        };
        let token = token_from(pair, Some("crsr_key")).unwrap();
        assert_eq!(token.expires_at, Some(100));
        assert_eq!(token.refresh_token.as_deref(), Some("crsr_key"));
    }

    #[test]
    fn rotated_refresh_jwt_replaces_the_old_one() {
        let pair = TokenPair {
            access_token: jwt(100),
            refresh_token: Some(jwt(999)),
        };
        let token = token_from(pair, Some("old.refresh.jwt")).unwrap();
        assert_eq!(token.refresh_token, Some(jwt(999)));
    }

    #[test]
    fn access_token_must_be_a_jwt() {
        let pair = TokenPair {
            access_token: "opaque".into(),
            refresh_token: None,
        };
        assert!(token_from(pair, None).is_err());
    }
}
