//! Proxy start/stop control and port display.

#[allow(clippy::wildcard_imports)]
use gpui::*;
use gpui_component::ActiveTheme as _;

#[derive(IntoElement)]
pub struct ProxyControl {
    running: bool,
    port: u16,
}

impl ProxyControl {
    pub fn new(running: bool, port: u16) -> Self {
        Self { running, port }
    }
}

impl RenderOnce for ProxyControl {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let label: SharedString = if self.running {
            "Stop Proxy".into()
        } else {
            "Start Proxy".into()
        };

        let port_label: SharedString = format!("port: {}", self.port).into();

        let button_bg = if self.running {
            theme.danger
        } else {
            theme.primary
        };
        let button_text = if self.running {
            theme.danger_foreground
        } else {
            theme.primary_foreground
        };

        div()
            .flex()
            .items_center()
            .justify_between()
            .w_full()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px(px(12.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(button_bg)
                    .text_color(button_text)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child(port_label),
            )
    }
}
