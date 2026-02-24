//! BYOKEY desktop client — Tauri v2 system tray app.
//!
//! Creates a system tray icon and renders the control panel as a small popup
//! window. The proxy server runs in-process on a background Tokio task.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_store::SqliteTokenStore;
use byokey_types::{ProviderId, TokenState};
use tauri::Manager as _;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

// ── Shared application state ─────────────────────────────────────────────────

struct AppState {
    auth: Arc<AuthManager>,
    config: Arc<Config>,
    proxy_running: Arc<AtomicBool>,
    proxy_port: u16,
}

// ── Tauri command response types ─────────────────────────────────────────────

#[derive(serde::Serialize)]
struct ProviderStatusResponse {
    id: String,
    state: String,
}

#[derive(serde::Serialize)]
struct ProxyStatusResponse {
    running: bool,
    port: u16,
}

// ── Tauri commands ───────────────────────────────────────────────────────────

#[tauri::command]
async fn get_providers_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ProviderStatusResponse>, String> {
    let mut statuses = Vec::new();
    for provider in ProviderId::all() {
        let token_state = state.auth.token_state(provider).await;
        let state_str = match token_state {
            TokenState::Valid => "valid",
            TokenState::Expired => "expired",
            TokenState::Invalid => "not_authenticated",
        };
        statuses.push(ProviderStatusResponse {
            id: provider.to_string(),
            state: state_str.to_string(),
        });
    }
    Ok(statuses)
}

#[tauri::command]
async fn get_proxy_status(
    state: tauri::State<'_, AppState>,
) -> Result<ProxyStatusResponse, String> {
    Ok(ProxyStatusResponse {
        running: state.proxy_running.load(Ordering::Relaxed),
        port: state.proxy_port,
    })
}

#[tauri::command]
async fn toggle_proxy(state: tauri::State<'_, AppState>) -> Result<ProxyStatusResponse, String> {
    let was_running = state.proxy_running.load(Ordering::Relaxed);

    if was_running {
        state.proxy_running.store(false, Ordering::Relaxed);
    } else {
        let config = (*state.config).clone();
        let auth = Arc::clone(&state.auth);
        let running_flag = Arc::clone(&state.proxy_running);
        let port = state.proxy_port;

        running_flag.store(true, Ordering::Relaxed);

        tokio::spawn(async move {
            let addr = format!("{}:{}", config.host, port);
            let config_arc = Arc::new(arc_swap::ArcSwap::from_pointee((*config).clone()));
            let app_state = byokey_proxy::AppState::new(config_arc, auth);
            let app = byokey_proxy::make_router(app_state);

            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("failed to bind proxy on {addr}: {e}");
                    running_flag.store(false, Ordering::Relaxed);
                    return;
                }
            };

            eprintln!("byokey proxy listening on http://{addr}");
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("proxy server error: {e}");
            }
            running_flag.store(false, Ordering::Relaxed);
        });
    }

    Ok(ProxyStatusResponse {
        running: state.proxy_running.load(Ordering::Relaxed),
        port: state.proxy_port,
    })
}

#[tauri::command]
async fn login_provider(
    provider: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let provider_id: ProviderId = provider.parse().map_err(|e: byokey_types::ByokError| e.to_string())?;
    byokey_auth::flow::login(&provider_id, &state.auth, None)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn logout_provider(
    provider: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let provider_id: ProviderId = provider.parse().map_err(|e: byokey_types::ByokError| e.to_string())?;
    state
        .auth
        .remove_token(&provider_id)
        .await
        .map_err(|e| e.to_string())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn load_config() -> Config {
    let config_path = dirs_config_path();
    if config_path.exists() {
        Config::from_file(&config_path).unwrap_or_default()
    } else {
        Config::default()
    }
}

async fn open_store() -> anyhow::Result<SqliteTokenStore> {
    let path = default_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite://{}", path.display());
    SqliteTokenStore::new(&url)
        .await
        .map_err(|e| anyhow::anyhow!("database error: {e}"))
}

fn default_db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("tokens.db")
}

fn dirs_config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".byokey").join("config.yaml")
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let config = Arc::new(load_config());
    let store = rt
        .block_on(open_store())
        .expect("failed to open token store");
    let auth = Arc::new(AuthManager::new(Arc::new(store), rquest::Client::new()));

    let app_state = AppState {
        auth,
        config: config.clone(),
        proxy_running: Arc::new(AtomicBool::new(false)),
        proxy_port: config.port,
    };

    let _guard = rt.enter();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_providers_status,
            get_proxy_status,
            toggle_proxy,
            login_provider,
            logout_provider,
        ])
        .setup(|app| {
            use tauri::menu::{MenuBuilder, MenuItemBuilder};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

            let show = MenuItemBuilder::with_id("show", "Show Panel").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit BYOKEY").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show)
                .separator()
                .item(&quit)
                .build()?;

            TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("BYOKEY")
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => {
                        std::process::exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            if window.is_visible().unwrap_or(false) {
                                let _ = window.hide();
                            } else {
                                let _ = window.show();
                                let _ = window.set_focus();
                            }
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
