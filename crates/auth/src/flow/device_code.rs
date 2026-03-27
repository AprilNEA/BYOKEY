//! Device Authorization Grant login flows.
//!
//! Handles providers that use a device code + polling pattern:
//! Copilot, Qwen, Kimi.

use byokey_types::{ByokError, ProviderId, traits::Result};
use std::time::Duration;

use super::{open_browser, save_login_token};
use crate::{
    AuthManager, credentials, pkce,
    provider::{copilot, kimi, qwen},
    token,
};

// ── Copilot device code flow ──────────────────────────────────────────────────

pub async fn login_copilot(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("copilot", http).await?;
    let device_code_url = creds
        .device_code_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("copilot credentials missing device_code_url".into()))?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("copilot credentials missing token_url".into()))?;
    let scope_str = copilot::SCOPES.join(" ");
    let init_params = [
        ("client_id", creds.client_id.as_str()),
        ("scope", scope_str.as_str()),
    ];

    let resp = http
        .post(device_code_url)
        .header("Accept", "application/json")
        .form(&init_params)
        .send()
        .await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = copilot::parse_device_code_response(&json)?;

    tracing::info!(uri = %dc.verification_uri, code = %dc.user_code, "visit URL and enter verification code");
    let _ = open::that(&dc.verification_uri);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
    let mut interval = dc.interval;
    let device_code = dc.device_code.clone();

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("device code expired".into()));
        }

        let token_params = [
            ("client_id", creds.client_id.as_str()),
            ("device_code", device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];

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

        match json.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            Some(e) => return Err(ByokError::Auth(format!("device flow error: {e}"))),
            None => {}
        }

        let tok = token::parse_token_response(&json)?;
        save_login_token(auth, &ProviderId::Copilot, tok, account).await?;
        tracing::info!("Copilot login successful");
        return Ok(());
    }
}

// ── Qwen device code + PKCE flow ──────────────────────────────────────────────

pub async fn login_qwen(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("qwen", http).await?;
    let device_code_url = creds
        .device_code_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("qwen credentials missing device_code_url".into()))?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("qwen credentials missing token_url".into()))?;
    let (verifier, challenge) = pkce::generate_pkce();
    let scope_str = qwen::SCOPES.join(" ");
    let device_params = qwen::build_device_code_params(&creds.client_id, &challenge, &scope_str);

    let resp = http
        .post(device_code_url)
        .header("Accept", "application/json")
        .form(&device_params)
        .send()
        .await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = qwen::parse_device_code_response(&json)?;

    tracing::info!(uri = %dc.verification_uri, code = %dc.user_code, "visit URL and enter verification code");
    let _ = open::that(&dc.verification_uri);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
    #[allow(clippy::cast_precision_loss)]
    let mut interval_secs = dc.interval as f64;
    let device_code = dc.device_code.clone();

    loop {
        tokio::time::sleep(Duration::from_secs_f64(interval_secs)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("device code expired".into()));
        }

        let token_params = qwen::build_token_poll_params(&creds.client_id, &device_code, &verifier);
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

        match json.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval_secs *= qwen::SLOW_DOWN_MULTIPLIER;
                continue;
            }
            Some(e) => return Err(ByokError::Auth(format!("device flow error: {e}"))),
            None => {}
        }

        let tok = token::parse_token_response(&json)?;
        save_login_token(auth, &ProviderId::Qwen, tok, account).await?;
        tracing::info!("Qwen login successful");
        return Ok(());
    }
}

// ── Kimi device code flow ─────────────────────────────────────────────────────

pub async fn login_kimi(
    auth: &AuthManager,
    http: &rquest::Client,
    account: Option<&str>,
) -> Result<()> {
    let creds = credentials::fetch("kimi", http).await?;
    let device_code_url = creds
        .device_code_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("kimi credentials missing device_code_url".into()))?;
    let token_url = creds
        .token_url
        .as_deref()
        .ok_or_else(|| ByokError::Auth("kimi credentials missing token_url".into()))?;
    let scope_str = kimi::SCOPES.join(" ");
    let device_params = kimi::build_device_code_params(&creds.client_id, &scope_str);
    let msh_headers = kimi::x_msh_headers();

    let mut req = http
        .post(device_code_url)
        .header("Accept", "application/json")
        .form(&device_params);
    for (name, value) in &msh_headers {
        req = req.header(*name, value.as_str());
    }

    let resp = req.send().await?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = kimi::parse_device_code_response(&json)?;

    tracing::info!(uri = %dc.verification_uri, code = %dc.user_code, "visit URL and enter verification code");
    open_browser(&dc.verification_uri);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
    let mut interval = dc.interval;
    let device_code = dc.device_code.clone();
    let poll_headers = kimi::x_msh_headers();

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("device code expired".into()));
        }

        let token_params = kimi::build_token_poll_params(&creds.client_id, &device_code);
        let mut req = http
            .post(token_url)
            .header("Accept", "application/json")
            .form(&token_params);
        for (name, value) in &poll_headers {
            req = req.header(*name, value.as_str());
        }

        let resp = req.send().await?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

        match json.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval += 5;
                continue;
            }
            Some(e) => return Err(ByokError::Auth(format!("device flow error: {e}"))),
            None => {}
        }

        let tok = token::parse_token_response(&json)?;
        save_login_token(auth, &ProviderId::Kimi, tok, account).await?;
        tracing::info!("Kimi login successful");
        return Ok(());
    }
}
