//! Live integration tests against the real `ampcode.com` API.
//!
//! These tests require a valid Amp CLI token at `~/.local/share/amp/secrets.json`.
//! Run with: `cargo test -p ampcode --test live`

#![allow(clippy::items_after_statements)]

use ampcode::{AmpcodeClient, Plan};

async fn live_client() -> AmpcodeClient {
    let token = ampcode::secrets::load_token()
        .await
        .expect("no amp token found — is Amp CLI logged in?");
    AmpcodeClient::new(token)
}

#[tokio::test]
async fn live_balance() {
    let client = live_client().await;

    // First, dump the raw RPC response to see the structure.
    let raw: serde_json::Value = client.rpc("userDisplayBalanceInfo", None).await.unwrap();
    println!(
        "raw response: {}",
        serde_json::to_string_pretty(&raw).unwrap()
    );

    // Raw display text should always work.
    let text = client.balance_display_text().await.unwrap();
    println!("display_text: {text}");
    assert!(!text.is_empty());

    // Try structured parse — print result either way.
    match client.balance().await {
        Ok(info) => {
            println!("plan: {:?}", info.plan);
            println!("user: {:?}", info.user);
            println!("remaining: {:?}", info.remaining_dollars);
            println!("total: {:?}", info.total_dollars);
            println!("rate: {:?}", info.replenish_rate_dollars);
            println!("credits: {:?}", info.credits_dollars);
            println!(
                "bonus: {:?}% for {:?} days",
                info.bonus_percent, info.bonus_days_remaining
            );

            match info.plan {
                Plan::Free => {
                    assert!(info.remaining_dollars.is_some());
                    assert!(info.total_dollars.is_some());
                    assert!(info.replenish_rate_dollars.is_some());
                }
                Plan::IndividualCredits => {
                    assert!(info.credits_dollars.is_some());
                }
                _ => {}
            }
        }
        Err(e) => {
            println!("balance parse failed (new format?): {e}");
            println!("raw text was: {text}");
        }
    }
}

#[tokio::test]
async fn live_local_threads() {
    let summaries = ampcode::local::list_thread_summaries().await.unwrap();
    println!("{} local threads found", summaries.len());

    if let Some(first) = summaries.first() {
        println!(
            "newest: {} — {:?} ({})",
            first.id,
            first.title.as_deref().unwrap_or("(untitled)"),
            first.agent_mode.as_deref().unwrap_or("?"),
        );

        // Read full thread.
        let thread = ampcode::local::read_thread_by_id(&first.id).unwrap();
        println!("  v={}, {} messages", thread.v, thread.messages.len());

        for msg in &thread.messages {
            let token_info = msg.usage.as_ref().map_or_else(String::new, |u| {
                format!(
                    " [{}; in={} out={}]",
                    u.model,
                    u.input_tokens.unwrap_or(0),
                    u.output_tokens.unwrap_or(0),
                )
            });
            println!("  msg#{} role={}{token_info}", msg.message_id, msg.role);
        }
    }
}

#[tokio::test]
async fn live_github_auth_status() {
    let client = live_client().await;
    match client.github_auth_status().await {
        Ok(status) => println!("github auth status: {status}"),
        Err(e) => println!("github auth status failed: {e}"),
    }
}
