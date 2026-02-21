//! System tray icon setup via `tray-icon`.
//!
//! Creates an `NSStatusItem` with a context menu and returns a channel
//! receiver for tray click events.

use crossbeam_channel::Receiver;
use tray_icon::{
    TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};

/// Set up the system tray icon and return a receiver for tray events.
///
/// The context menu contains:
/// - Show Panel
/// - Quit
pub fn setup() -> Receiver<TrayIconEvent> {
    let (tx, rx) = crossbeam_channel::unbounded();

    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = tx.send(event);
    }));

    // Build context menu.
    let menu = Menu::new();
    let show_item = MenuItem::new("Show Panel", true, None);
    let quit_item = MenuItem::new("Quit BYOKEY", true, None);
    let quit_id = quit_item.id().clone();

    menu.append(&show_item).expect("failed to add menu item");
    menu.append(&quit_item).expect("failed to add menu item");

    // Handle menu events (quit).
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == quit_id {
            std::process::exit(0);
        }
    }));

    // Build tray icon with a tooltip; icon image is optional for now.
    let tray = TrayIconBuilder::new()
        .with_tooltip("BYOKEY")
        .with_menu(Box::new(menu))
        .build()
        .expect("failed to create tray icon");

    // Leak the tray icon to keep it alive for the app lifetime.
    std::mem::forget(tray);

    rx
}
