//! BYOKEY desktop: a management window for the local BYOKEY server.

#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod accounts;
mod brew;
mod gate;
mod overview;
mod server;
mod shell;
mod updater;

use gpui_kit::component::{ActiveTheme as _, Root, Theme};
use gpui_kit::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, Styled as _, TitlebarOptions,
    WindowBounds, WindowHandle, WindowOptions, actions, px, size,
};

use crate::server::Server;
use crate::shell::Shell;

actions!(byokey, [Quit]);

const APP_ID: &str = "io.byokey.desktop";

struct MainWindow(WindowHandle<Root>);

impl gpui_kit::Global for MainWindow {}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let server = Server::from_env()?;
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    app.on_reopen(open_window);
    app.run(move |cx| {
        gpui_kit::init(cx);
        cx.set_global(server);
        updater::install(cx);

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new("secondary-q", Quit, None)]);
        cx.set_menus(vec![
            Menu::new("BYOKEY").items(vec![MenuItem::action("Quit BYOKEY", Quit)]),
        ]);

        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        open_window(cx);
    });
    Ok(())
}

/// Open the main window, or focus it when it is already open.
fn open_window(cx: &mut App) {
    if let Some(handle) = cx.try_global::<MainWindow>().map(|main| main.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        cx.activate(true);
        return;
    }

    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("BYOKEY".into()),
            ..TitlebarOptions::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(960.), px(640.)),
            cx,
        ))),
        window_min_size: Some(size(px(720.), px(480.))),
        app_id: Some(APP_ID.into()),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        Theme::sync_system_appearance(Some(window), cx);
        let shell = cx.new(|cx| Shell::new(window, cx));
        cx.new(|cx| Root::new(shell, window, cx).bg(cx.theme().background))
    });
    match opened {
        Ok(handle) => {
            cx.set_global(MainWindow(handle));
            cx.activate(true);
        }
        Err(e) => tracing::error!(error = %e, "could not open the main window"),
    }
}
