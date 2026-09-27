//! The main window: connects, applies the [`Gate`], then shows the pages.

use std::time::Duration;

use byokey_proto::byokey::status as stat;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, px,
};
use gpui_updater::{UpdateStatus, Updater};

use crate::gate::Gate;
use crate::server::Server;
use crate::{accounts::AccountsPage, brew, overview::OverviewPage, updater};

const POLL: Duration = Duration::from_secs(5);

type UpdaterAction = fn(&mut Updater, &mut Context<Updater>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Accounts,
}

enum Connection {
    Connecting,
    Unreachable(String),
    Connected {
        server: stat::ServerInfo,
        gate: Gate,
    },
}

/// What the server update is doing, when this app drives it.
enum ServerUpdate {
    Idle,
    Running,
    Failed(String),
}

pub struct Shell {
    page: Page,
    connection: Connection,
    server_update: ServerUpdate,
    overview: Entity<OverviewPage>,
    accounts: Entity<AccountsPage>,
    updater: Entity<Updater>,
    _poll: Task<()>,
    _updater: Subscription,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let updater = updater::shared(cx);
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                let status = cx
                    .update(|_, cx| Server::call(cx, |c| async move { Ok(c.get_status().await?) }));
                let Ok(status) = status else { return };
                let result = status.await;
                if this
                    .update(cx, |this, cx| this.connected(result, cx))
                    .is_err()
                {
                    return;
                }
                cx.background_executor().timer(POLL).await;
            }
        });
        Self {
            page: Page::Overview,
            connection: Connection::Connecting,
            server_update: ServerUpdate::Idle,
            overview: cx.new(OverviewPage::new),
            accounts: cx.new(|cx| AccountsPage::new(window, cx)),
            _updater: cx.observe(&updater, |_, _, cx| cx.notify()),
            updater,
            _poll: poll,
        }
    }

    fn connected(
        &mut self,
        result: anyhow::Result<stat::GetStatusResponse>,
        cx: &mut Context<Self>,
    ) {
        self.connection = match result {
            Ok(status) => {
                let server = status.server.into_option().unwrap_or_default();
                let gate = Gate::classify(
                    server.api_version,
                    &server.version,
                    &updater::current_version(),
                );
                if gate.is_usable() {
                    self.overview
                        .update(cx, |page, cx| page.set_providers(status.providers, cx));
                    self.overview.update(cx, OverviewPage::refresh_usage);
                    self.accounts.update(cx, AccountsPage::refresh);
                }
                Connection::Connected { server, gate }
            }
            Err(e) => Connection::Unreachable(format!("{e:#}")),
        };
        cx.notify();
    }

    fn update_server(&mut self, brew: std::path::PathBuf, cx: &mut Context<Self>) {
        self.server_update = ServerUpdate::Running;
        cx.notify();
        let upgrade = cx
            .background_executor()
            .spawn(async move { brew::upgrade_server(&brew) });
        cx.spawn(async move |this, cx| {
            let result = upgrade.await;
            this.update(cx, |this, cx| {
                // The next poll picks up the restarted server.
                this.server_update = match result {
                    Ok(()) => ServerUpdate::Idle,
                    Err(e) => ServerUpdate::Failed(format!("{e:#}")),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let item = |label: &'static str, icon: IconName, page: Page| {
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(self.page == page)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.page = page;
                    cx.notify();
                }))
        };
        Sidebar::new("nav")
            .header(div().px_2().text_lg().child("BYOKEY"))
            .child(
                SidebarGroup::new("Server").child(
                    SidebarMenu::new()
                        .child(item("Overview", IconName::LayoutDashboard, Page::Overview))
                        .child(item("Accounts", IconName::CircleUser, Page::Accounts)),
                ),
            )
            .footer(self.render_app_update(cx))
    }

    fn render_app_update(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.updater.read(cx).status().clone();
        let updater = self.updater.clone();
        let (label, action): (SharedString, Option<UpdaterAction>) = match &status {
            UpdateStatus::Available(v) => (
                format!("Update to {v}").into(),
                Some(Updater::download_and_install),
            ),
            UpdateStatus::Downloading { downloaded, total } => (
                match total {
                    Some(total) if *total > 0 => {
                        format!("Downloading {}%", downloaded * 100 / total).into()
                    }
                    _ => "Downloading…".into(),
                },
                None,
            ),
            UpdateStatus::Installing => ("Installing…".into(), None),
            UpdateStatus::Staged(v) => (
                format!("Restart to {v}").into(),
                Some(|u, cx| u.restart(cx)),
            ),
            UpdateStatus::Checking => ("Checking…".into(), None),
            UpdateStatus::Idle | UpdateStatus::UpToDate | UpdateStatus::Errored(_) => {
                ("Check for updates".into(), Some(Updater::check))
            }
        };
        v_flex()
            .gap_1()
            .px_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(format!("App {}", updater::current_version()))
            .when_some(
                match &status {
                    UpdateStatus::Errored(e) => Some(e.clone()),
                    _ => None,
                },
                |el, e| el.child(div().text_color(cx.theme().danger).child(e)),
            )
            .child(
                Button::new("app-update")
                    .label(label)
                    .when(
                        matches!(status, UpdateStatus::Available(_) | UpdateStatus::Staged(_)),
                        ButtonVariants::primary,
                    )
                    .disabled(action.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(action) = action {
                            updater.update(cx, action);
                        }
                    }),
            )
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match &self.connection {
            Connection::Connecting => centered(Spinner::new()).into_any_element(),
            Connection::Unreachable(error) => notice(
                "BYOKEY isn't running",
                format!(
                    "The app talks to a BYOKEY server on this machine. Start it with \
                     `brew services start byokey` (or `byokey start`); this window \
                     connects as soon as it is up.\n\n{error}"
                ),
                cx,
            )
            .into_any_element(),
            Connection::Connected { server, gate } => match gate {
                Gate::ServerTooOld => self
                    .render_server_update(server, true, cx)
                    .into_any_element(),
                Gate::AppTooOld => notice(
                    "This app is too old for the server",
                    format!(
                        "BYOKEY {} speaks a newer management API. Update the app from the \
                         sidebar to continue.",
                        server.version
                    ),
                    cx,
                )
                .into_any_element(),
                Gate::ServerBehind { .. } | Gate::Ready => v_flex()
                    .size_full()
                    .when(matches!(gate, Gate::ServerBehind { .. }), |el| {
                        el.child(self.render_server_update(server, false, cx))
                    })
                    .child(div().flex_1().min_h_0().child(match self.page {
                        Page::Overview => self.overview.clone().into_any_element(),
                        Page::Accounts => self.accounts.clone().into_any_element(),
                    }))
                    .into_any_element(),
            },
        }
    }

    /// The server update, as a blocking page when `required`, else a banner.
    fn render_server_update(
        &self,
        server: &stat::ServerInfo,
        required: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let app = updater::current_version();
        let brew = brew::server_brew(&server.executable);
        let running = matches!(self.server_update, ServerUpdate::Running);
        let title: SharedString = if required {
            "BYOKEY needs an update".into()
        } else {
            format!("BYOKEY {} is older than this app ({app})", server.version).into()
        };
        let detail: SharedString = match (&brew, &self.server_update) {
            (_, ServerUpdate::Failed(e)) => e.clone().into(),
            (Some(_), _) => "Homebrew installed it; updating runs `brew upgrade byokey` and restarts its service.".into(),
            (None, _) => format!(
                "Update the server at {} the way you installed it, then restart it.",
                if server.executable.is_empty() { "its install location" } else { &server.executable }
            )
            .into(),
        };
        let button = brew.map(|brew| {
            Button::new("server-update")
                .primary()
                .label(if running {
                    "Updating…"
                } else {
                    "Update BYOKEY"
                })
                .loading(running)
                .disabled(running)
                .on_click(cx.listener(move |this, _, _, cx| this.update_server(brew.clone(), cx)))
        });
        let body = v_flex()
            .gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            )
            .children(button);
        if required {
            centered(body.max_w(px(480.))).into_any_element()
        } else {
            h_flex()
                .p_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .child(body)
                .into_any_element()
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dialogs = gpui_kit::component::Root::render_dialog_layer(window, cx);
        let notifications = gpui_kit::component::Root::render_notification_layer(window, cx);
        h_flex()
            .size_full()
            .child(self.render_sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.render_body(cx)),
            )
            .children(dialogs)
            .children(notifications)
    }
}

fn centered(child: impl IntoElement) -> gpui_kit::Div {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

fn notice(title: &'static str, detail: String, cx: &App) -> impl IntoElement {
    centered(
        v_flex()
            .max_w(px(480.))
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(detail),
            ),
    )
}
