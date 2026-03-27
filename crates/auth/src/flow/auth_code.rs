//! Authorization Code + PKCE login flows.
//!
//! Handles providers that use a browser-based redirect with a local callback
//! server: Claude, Codex, Gemini, Antigravity, iFlow.

use byokey_types::{ByokError, OAuthToken, ProviderId, traits::Result};

use super::{open_browser, save_login_token};
use crate::{
    AuthManager, callback, credentials, pkce,
    provider::{antigravity, claude, codex, gemini, iflow},
    token,
};

// ── Claude PKCE flow ──────────────────────────────────────────────────────────

pub async fn login_claude(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("claude", http).await?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("claude credentials missing token_url".into()))?;
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = claude::build_auth_url(&creds.client_id, &challenge, &state);

    let listener = callback::bind_callback(claude::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;
    verify_state(&params, &state)?;

    let code = extract_code(&params)?;
    let body = claude::build_token_request(&creds.client_id, code, &verifier, &state);
    let resp = http
        .post(token_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let tok = token::parse_token_response(&json)?;
    save_login_token(auth, &ProviderId::Claude, tok, account).await?;
    tracing::info!("Claude login successful");
    Ok(())
}

// ── Codex auth code flow ──────────────────────────────────────────────────────

pub async fn login_codex(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("codex", http).await?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("codex credentials missing token_url".into()))?;
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = codex::build_auth_url(&creds.client_id, &challenge, &state);

    open_browser(&auth_url);

    let params = callback::wait_for_callback(codex::CALLBACK_PORT).await?;
    verify_state(&params, &state)?;

    let code = extract_code(&params)?;
    let token_params = codex::token_form_params(&creds.client_id, code, &verifier);
    let resp = http
        .post(token_url)
        .header("Accept", "application/json")
        .form(&token_params)
        .send()
        .await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let tok = token::parse_token_response(&json)?;
    save_login_token(auth, &ProviderId::Codex, tok, account).await?;
    tracing::info!("Codex login successful");
    Ok(())
}

// ── Gemini PKCE flow ──────────────────────────────────────────────────────────

pub async fn login_gemini(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("gemini", http).await?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("gemini credentials missing token_url".into()))?;
    let client_secret = creds
        .client_secret
        .as_deref()
        .ok_or_else(|| ByokError::Auth("gemini credentials missing client_secret".into()))?;

    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = gemini::build_auth_url(&creds.client_id, &challenge, &state);

    let listener = callback::bind_callback(gemini::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;
    verify_state(&params, &state)?;

    let code = extract_code(&params)?;
    let token_params = gemini::token_form_params(&creds.client_id, client_secret, code, &verifier);
    let resp = http.post(token_url).form(&token_params).send().await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let tok = token::parse_token_response(&json)?;
    save_login_token(auth, &ProviderId::Gemini, tok, account).await?;
    tracing::info!("Gemini login successful");
    Ok(())
}

// ── Antigravity (Google Cloud Code Assist) PKCE flow ─────────────────────────

pub async fn login_antigravity(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("antigravity", http).await?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("antigravity credentials missing token_url".into()))?;
    let client_secret = creds
        .client_secret
        .as_deref()
        .ok_or_else(|| ByokError::Auth("antigravity credentials missing client_secret".into()))?;

    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = antigravity::build_auth_url(&creds.client_id, &challenge, &state);

    let listener = callback::bind_callback(antigravity::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;
    verify_state(&params, &state)?;

    let code = extract_code(&params)?;
    let token_params =
        antigravity::token_form_params(&creds.client_id, client_secret, code, &verifier);
    let resp = http.post(token_url).form(&token_params).send().await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let tok = token::parse_token_response(&json)?;
    save_login_token(auth, &ProviderId::Antigravity, tok, account).await?;
    tracing::info!("Antigravity login successful");
    Ok(())
}

// ── iFlow (Z.ai / GLM) auth code flow ────────────────────────────────────────

pub async fn login_iflow(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("iflow", http).await?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("iflow credentials missing token_url".into()))?;
    let client_secret = creds
        .client_secret
        .as_deref()
        .ok_or_else(|| ByokError::Auth("iflow credentials missing client_secret".into()))?;

    let state = pkce::random_state();
    let auth_url = iflow::build_auth_url(&creds.client_id, &state);

    let listener = callback::bind_callback(iflow::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;
    verify_state(&params, &state)?;

    let code = extract_code(&params)?;
    let token_params = iflow::token_form_params(&creds.client_id, code);
    let resp = http
        .post(token_url)
        .header(
            "Authorization",
            iflow::basic_auth_header(&creds.client_id, client_secret),
        )
        .form(&token_params)
        .send()
        .await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let tok = token::parse_token_response(&json)?;

    // Exchange the OAuth access_token for an iFlow API key and store it as
    // the token's access_token so the executor can use it directly.
    let oauth_access = tok.access_token.clone();
    let api_key = iflow::fetch_api_key(&oauth_access, http).await?;
    let tok = OAuthToken {
        access_token: api_key,
        ..tok
    };

    save_login_token(auth, &ProviderId::IFlow, tok, account).await?;
    tracing::info!("iFlow (Z.ai/GLM) login successful");
    Ok(())
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn verify_state(params: &std::collections::HashMap<String, String>, expected: &str) -> Result<()> {
    let received = params.get("state").map_or("", String::as_str);
    if received != expected {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }
    Ok(())
}

fn extract_code(params: &std::collections::HashMap<String, String>) -> Result<&str> {
    params
        .get("code")
        .map(String::as_str)
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))
}
