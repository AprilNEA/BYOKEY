//! BYOKEY desktop client — macOS menu bar app.
//!
//! Creates a system tray icon via `tray-icon` and renders the control panel
//! as a GPUI popup window.

mod components;
mod panel;
mod tray;

#[allow(clippy::wildcard_imports)]
use gpui::*;

use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_store::SqliteTokenStore;
use gpui_component::Root;
use panel::PanelView;
use std::sync::Arc;

fn main() {
    // Create the Tokio runtime before GPUI starts.
    //
    // sqlx (used by SqliteTokenStore) calls `tokio::runtime::Handle::current()` when
    // executing queries. GPUI dispatches async tasks through the macOS Cocoa run loop
    // (trampoline), which runs on the main thread but outside any Tokio context. By
    // creating the runtime here and entering it with `_tokio_guard`, the thread-local
    // Tokio context remains active on the main thread for the lifetime of `app.run()`.
    //
    // The runtime's multi-threaded worker pool processes sqlx blocking I/O in the
    // background; `app.run()` blocks main(), so `rt` is never dropped while the app runs.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let config = Arc::new(load_config());
    let store = rt.block_on(open_store()).expect("failed to open token store");
    let auth = Arc::new(AuthManager::new(Arc::new(store)));

    let app = Application::new();

    // Enter the Tokio runtime context on the main thread and keep the guard alive
    // for the duration of `app.run()`. Variables declared with a leading underscore
    // but with a full name (not `_`) stay alive until end of scope.
    let _tokio_guard = rt.enter();

    app.run(move |cx| {
        gpui_component::init(cx);

        // Set up system tray.
        let tray_rx = tray::setup();

        let auth_handle = Arc::clone(&auth);
        let config_handle = Arc::clone(&config);

        cx.spawn(async move |cx| {
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Point::default(),
                            size: size(px(320.0), px(460.0)),
                        })),
                        kind: WindowKind::PopUp,
                        is_movable: false,
                        focus: true,
                        show: false,
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|cx| PanelView::new(auth_handle, config_handle, cx));
                        let view_any: AnyView = view.into();
                        cx.new(|cx| Root::new(view_any, window, cx))
                    },
                )
                .expect("failed to open panel window");

            // Poll tray events and toggle window visibility.
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                while let Ok(event) = tray_rx.try_recv() {
                    if let tray_icon::TrayIconEvent::Click { .. } = event {
                        window
                            .update(cx, |_root, window, _cx| {
                                if window.is_window_active() {
                                    window.minimize_window();
                                } else {
                                    window.activate_window();
                                }
                            })
                            .ok();
                    }
                }
            }
        })
        .detach();
    });
}

fn load_config() -> Config {
    let config_path = dirs_config_path();
    if config_path.exists() {
        Config::from_file(&config_path).unwrap_or_default()
    } else {
        Config::default()
    }
}

async fn open_store() -> Result<SqliteTokenStore> {
    let path = default_db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite://{}", path.display());
    SqliteTokenStore::new(&url)
        .await
        .map_err(|e| anyhow::anyhow!("database error: {e}"))
}

fn default_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".byokey")
        .join("tokens.db")
}

fn dirs_config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".byokey")
        .join("config.yaml")
}
