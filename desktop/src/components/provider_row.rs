//! A single row in the provider list showing name, status, and login action.

use crate::panel::{ProviderStatus, TokenStatusKind};
#[allow(clippy::wildcard_imports)]
use gpui::*;
use gpui_component::ActiveTheme as _;

#[derive(IntoElement)]
pub struct ProviderRow {
    status: ProviderStatus,
}

impl ProviderRow {
    pub fn new(status: ProviderStatus) -> Self {
        Self { status }
    }

    fn status_icon(&self) -> &'static str {
        match self.status.state {
            TokenStatusKind::Valid => "✓",
            TokenStatusKind::Expired => "○",
            TokenStatusKind::NotAuthenticated => "✗",
        }
    }

    fn status_label(&self) -> SharedString {
        match self.status.state {
            TokenStatusKind::Valid => "authenticated".into(),
            TokenStatusKind::Expired => "expired".into(),
            TokenStatusKind::NotAuthenticated => "not logged in".into(),
        }
    }
}

impl RenderOnce for ProviderRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();

        let icon_color = match self.status.state {
            TokenStatusKind::Valid => theme.success,
            TokenStatusKind::Expired => theme.warning,
            TokenStatusKind::NotAuthenticated => theme.muted_foreground,
        };

        let name: SharedString = self.status.id.to_string().into();

        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .py(px(6.0))
            .px(px(4.0))
            .rounded(px(6.0))
            .hover(|s| s.bg(theme.muted))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .text_color(icon_color)
                            .child(self.status_icon()),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child(self.status_label()),
            )
    }
}
