//! Interactive login flow dispatcher for all supported providers.

use byok_types::{ByokError, ProviderId, traits::Result};
use std::time::Duration;

use crate::{
    AuthManager, antigravity, callback, claude, codex, copilot, gemini, iflow, kimi, pkce, qwen,
};

/// Run the full interactive login flow for the given provider.
///
/// # Errors
///
/// Returns an error if the login flow fails for any reason (e.g., network error,
/// state mismatch, missing callback parameters, or token parse failure).
pub async fn login(provider: &ProviderId, auth: &AuthManager) -> Result<()> {
    let http = rquest::Client::new();
    match provider {
        ProviderId::Claude => login_claude(auth, &http).await,
        ProviderId::Codex => login_codex(auth, &http).await,
        ProviderId::Copilot => login_copilot(auth, &http).await,
        ProviderId::Gemini => login_gemini(auth, &http).await,
        ProviderId::Antigravity => login_antigravity(auth, &http).await,
        ProviderId::Qwen => login_qwen(auth, &http).await,
        ProviderId::Kimi => login_kimi(auth, &http).await,
        ProviderId::IFlow => login_iflow(auth, &http).await,
        ProviderId::Kiro => Err(ByokError::Auth(
            "Kiro OAuth login not yet implemented".into(),
        )),
    }
}

// ── Claude PKCE flow ──────────────────────────────────────────────────────────

async fn login_claude(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = claude::build_auth_url(&challenge, &state);

    let listener = callback::bind_callback(claude::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;

    let received_state = params.get("state").map_or("", String::as_str);
    if received_state != state {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))?;

    let body = claude::build_token_request(code, &verifier, &state);
    let resp = http
        .post(claude::TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let token = claude::parse_token_response(&json)?;
    auth.save_token(&ProviderId::Claude, token).await?;
    eprintln!("Claude login successful");
    Ok(())
}

// ── Codex auth code flow ──────────────────────────────────────────────────────

async fn login_codex(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = codex::build_auth_url(&challenge, &state);

    open_browser(&auth_url);

    let params = callback::wait_for_callback(codex::CALLBACK_PORT).await?;

    let received_state = params.get("state").map_or("", String::as_str);
    if received_state != state {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))?;

    let token_params = codex::token_form_params(code, &verifier);
    let resp = http
        .post(codex::TOKEN_URL)
        .header("Accept", "application/json")
        .form(&token_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let token = codex::parse_token_response(&json)?;
    auth.save_token(&ProviderId::Codex, token).await?;
    eprintln!("Codex login successful");
    Ok(())
}

// ── Copilot device code flow ──────────────────────────────────────────────────

async fn login_copilot(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let scope_str = copilot::SCOPES.join(" ");
    let init_params = [
        ("client_id", copilot::CLIENT_ID),
        ("scope", scope_str.as_str()),
    ];

    let resp = http
        .post(copilot::DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&init_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = copilot::parse_device_code_response(&json)?;

    eprintln!("Visit: {}", dc.verification_uri);
    eprintln!("Enter verification code: {}", dc.user_code);
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
            ("client_id", copilot::CLIENT_ID),
            ("device_code", device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];

        let resp = http
            .post(copilot::TOKEN_URL)
            .header("Accept", "application/json")
            .form(&token_params)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

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

        let token = copilot::parse_token_response(&json)?;
        auth.save_token(&ProviderId::Copilot, token).await?;
        eprintln!("Copilot login successful");
        return Ok(());
    }
}

// ── Gemini PKCE flow ──────────────────────────────────────────────────────────

async fn login_gemini(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = gemini::build_auth_url(&challenge, &state);

    let listener = callback::bind_callback(gemini::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;

    let received_state = params.get("state").map_or("", String::as_str);
    if received_state != state {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))?;

    let token_params = gemini::token_form_params(code, &verifier);
    let resp = http
        .post(gemini::TOKEN_URL)
        .form(&token_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let token = gemini::parse_token_response(&json)?;
    auth.save_token(&ProviderId::Gemini, token).await?;
    eprintln!("Gemini login successful");
    Ok(())
}

// ── Antigravity (Google Cloud Code Assist) PKCE flow ─────────────────────────

async fn login_antigravity(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let (verifier, challenge) = pkce::generate_pkce();
    let state = pkce::random_state();
    let auth_url = antigravity::build_auth_url(&challenge, &state);

    let listener = callback::bind_callback(antigravity::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;

    let received_state = params.get("state").map_or("", String::as_str);
    if received_state != state {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))?;

    let token_params = antigravity::token_form_params(code, &verifier);
    let resp = http
        .post(antigravity::TOKEN_URL)
        .form(&token_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let token = antigravity::parse_token_response(&json)?;
    auth.save_token(&ProviderId::Antigravity, token).await?;
    eprintln!("Antigravity login successful");
    Ok(())
}

// ── Qwen device code + PKCE flow ──────────────────────────────────────────────

async fn login_qwen(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let (verifier, challenge) = pkce::generate_pkce();
    let scope_str = qwen::SCOPES.join(" ");
    let device_params = qwen::build_device_code_params(&challenge, &scope_str);

    let resp = http
        .post(qwen::DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&device_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = qwen::parse_device_code_response(&json)?;

    eprintln!("Visit: {}", dc.verification_uri);
    eprintln!("Enter verification code: {}", dc.user_code);
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

        let token_params = qwen::build_token_poll_params(&device_code, &verifier);
        let resp = http
            .post(qwen::TOKEN_URL)
            .header("Accept", "application/json")
            .form(&token_params)
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

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

        let token = qwen::parse_token_response(&json)?;
        auth.save_token(&ProviderId::Qwen, token).await?;
        eprintln!("Qwen login successful");
        return Ok(());
    }
}

// ── Kimi device code flow ─────────────────────────────────────────────────────

async fn login_kimi(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let scope_str = kimi::SCOPES.join(" ");
    let device_params = kimi::build_device_code_params(&scope_str);
    let msh_headers = kimi::x_msh_headers();

    let mut req = http
        .post(kimi::DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&device_params);
    for (name, value) in &msh_headers {
        req = req.header(*name, value.as_str());
    }

    let resp = req
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse device code response: {e}")))?;

    let dc = kimi::parse_device_code_response(&json)?;

    eprintln!("Visit: {}", dc.verification_uri);
    eprintln!("Enter verification code: {}", dc.user_code);
    let _ = open::that(&dc.verification_uri);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(dc.expires_in);
    let mut interval = dc.interval;
    let device_code = dc.device_code.clone();
    let poll_headers = kimi::x_msh_headers();

    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;

        if tokio::time::Instant::now() >= deadline {
            return Err(ByokError::Auth("device code expired".into()));
        }

        let token_params = kimi::build_token_poll_params(&device_code);
        let mut req = http
            .post(kimi::TOKEN_URL)
            .header("Accept", "application/json")
            .form(&token_params);
        for (name, value) in &poll_headers {
            req = req.header(*name, value.as_str());
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ByokError::Http(e.to_string()))?;

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

        let token = kimi::parse_token_response(&json)?;
        auth.save_token(&ProviderId::Kimi, token).await?;
        eprintln!("Kimi login successful");
        return Ok(());
    }
}

// ── iFlow (Z.ai / GLM) auth code flow ────────────────────────────────────────

async fn login_iflow(auth: &AuthManager, http: &rquest::Client) -> Result<()> {
    let state = pkce::random_state();
    let auth_url = iflow::build_auth_url(&state);

    let listener = callback::bind_callback(iflow::CALLBACK_PORT).await?;
    open_browser(&auth_url);

    let params = callback::accept_callback(listener).await?;

    let received_state = params.get("state").map_or("", String::as_str);
    if received_state != state {
        return Err(ByokError::Auth(
            "state mismatch, possible CSRF attack".into(),
        ));
    }

    let code = params
        .get("code")
        .ok_or_else(|| ByokError::Auth("missing code parameter in callback".into()))?;

    let token_params = iflow::token_form_params(code);
    let resp = http
        .post(iflow::TOKEN_URL)
        .header("Authorization", iflow::basic_auth_header())
        .form(&token_params)
        .send()
        .await
        .map_err(|e| ByokError::Http(e.to_string()))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ByokError::Auth(format!("failed to parse token response: {e}")))?;

    let token = iflow::parse_token_response(&json)?;
    auth.save_token(&ProviderId::IFlow, token).await?;
    eprintln!("iFlow (Z.ai/GLM) login successful");
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn open_browser(url: &str) {
    eprintln!("Opening browser: {url}");
    if let Err(e) = open::that(url) {
        eprintln!("Failed to open browser automatically: {e}");
        eprintln!("Please open the following URL manually to complete login:");
        eprintln!("{url}");
    }
}
