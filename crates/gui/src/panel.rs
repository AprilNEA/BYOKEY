//! Main popup panel view.
//!
//! This is the GPUI view that renders the provider dashboard and proxy controls
//! inside the popup window shown when clicking the tray icon.

use crate::components::{ProviderRow, ProxyControl};
use byokey_auth::AuthManager;
use byokey_config::Config;
use byokey_types::{ProviderId, TokenState};
#[allow(clippy::wildcard_imports)]
use gpui::*;
use gpui_component::ActiveTheme as _;
use std::sync::Arc;

/// Per-provider display state.
#[derive(Clone)]
pub struct ProviderStatus {
    pub id: ProviderId,
    pub state: TokenStatusKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TokenStatusKind {
    Valid,
    Expired,
    NotAuthenticated,
}

/// The main panel view shown in the popup window.
pub struct PanelView {
    auth: Arc<AuthManager>,
    #[allow(dead_code)]
    config: Arc<Config>,
    providers: Vec<ProviderStatus>,
    proxy_running: bool,
    port: u16,
}

impl PanelView {
    pub fn new(auth: Arc<AuthManager>, config: Arc<Config>, cx: &mut Context<Self>) -> Self {
        let port = config.port;
        let mut view = Self {
            auth,
            config,
            providers: Vec::new(),
            proxy_running: false,
            port,
        };
        view.refresh_status(cx);
        view
    }

    /// Refresh provider authentication status from the token store.
    fn refresh_status(&mut self, cx: &mut Context<Self>) {
        let auth = Arc::clone(&self.auth);
        cx.spawn(async move |this, cx| {
            let mut statuses = Vec::new();
            for id in ProviderId::all() {
                let kind = match auth.token_state(id).await {
                    TokenState::Valid => TokenStatusKind::Valid,
                    TokenState::Expired => TokenStatusKind::Expired,
                    TokenState::Invalid => TokenStatusKind::NotAuthenticated,
                };
                statuses.push(ProviderStatus {
                    id: id.clone(),
                    state: kind,
                });
            }
            this.update(cx, |view, cx| {
                view.providers = statuses;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                // Header
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::BOLD)
                            .child("BYOKEY"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(status_dot(self.proxy_running, theme))
                            .child(div().text_size(px(12.0)).child(if self.proxy_running {
                                SharedString::from("Running")
                            } else {
                                SharedString::from("Stopped")
                            })),
                    ),
            )
            .child(
                // Provider list
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .px(px(16.0))
                    .py(px(8.0))
                    .gap(px(4.0))
                    .children(self.providers.iter().map(|p| ProviderRow::new(p.clone()))),
            )
            .child(
                // Footer — proxy control
                div()
                    .border_t_1()
                    .border_color(theme.border)
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(ProxyControl::new(self.proxy_running, self.port)),
            )
    }
}

fn status_dot(running: bool, theme: &gpui_component::theme::ThemeColor) -> Div {
    let color = if running {
        theme.success
    } else {
        theme.muted_foreground
    };
    div().size(px(8.0)).rounded(px(4.0)).bg(color)
}
