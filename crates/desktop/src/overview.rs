//! Provider state and usage counters.

use byokey_proto::byokey::status as stat;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div,
};

use crate::server::Server;

#[derive(Default)]
pub struct OverviewPage {
    providers: Vec<stat::ProviderStatus>,
    usage: Option<stat::GetUsageResponse>,
    error: Option<SharedString>,
}

impl OverviewPage {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self::default()
    }

    pub fn set_providers(&mut self, providers: Vec<stat::ProviderStatus>, cx: &mut Context<Self>) {
        self.providers = providers;
        cx.notify();
    }

    #[expect(
        clippy::unused_self,
        reason = "the receiver lets `Entity::update(cx, Self::refresh_usage)` take it by path"
    )]
    pub fn refresh_usage(&mut self, cx: &mut Context<Self>) {
        let usage = Server::call(cx, |c| async move { Ok(c.get_usage().await?) });
        cx.spawn(async move |this, cx| {
            let result = usage.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(usage) => {
                        this.usage = Some(usage);
                        this.error = None;
                    }
                    Err(e) => this.error = Some(format!("{e:#}").into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

fn auth_tag(status: Option<stat::AuthStatus>) -> Tag {
    match status {
        Some(stat::AuthStatus::AUTH_STATUS_VALID) => Tag::success().child("Signed in"),
        Some(stat::AuthStatus::AUTH_STATUS_EXPIRED) => Tag::warning().child("Expired"),
        _ => Tag::secondary().child("Not signed in"),
    }
}

impl Render for OverviewPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let providers = self.providers.iter().map(|p| {
            h_flex()
                .justify_between()
                .py_2()
                .border_b_1()
                .border_color(border)
                .child(div().child(p.display_name.clone()))
                .child(if p.enabled {
                    auth_tag(p.auth_status.as_known())
                } else {
                    Tag::secondary().outline().child("Disabled")
                })
        });

        let stat_card = |label: &'static str, value: u64| {
            v_flex()
                .flex_1()
                .p_3()
                .gap_1()
                .border_1()
                .border_color(border)
                .rounded_md()
                .child(div().text_xs().text_color(muted).child(label))
                .child(div().text_xl().child(value.to_string()))
        };

        let mut models: Vec<_> = self
            .usage
            .as_ref()
            .map(|u| u.models.iter().collect())
            .unwrap_or_default();
        models.sort_unstable_by_key(|(_, stats)| std::cmp::Reverse(stats.requests));

        v_flex()
            .id("overview")
            .size_full()
            .overflow_y_scroll()
            .p_5()
            .gap_5()
            .child(div().text_xl().child("Overview"))
            .child(
                v_flex()
                    .child(div().text_sm().text_color(muted).child("Providers"))
                    .children(providers),
            )
            .children(self.usage.as_ref().map(|u| {
                h_flex()
                    .gap_3()
                    .child(stat_card("Requests", u.total_requests))
                    .child(stat_card("Failed", u.failure_requests))
                    .child(stat_card("Input tokens", u.input_tokens))
                    .child(stat_card("Output tokens", u.output_tokens))
            }))
            .child(
                v_flex()
                    .child(div().text_sm().text_color(muted).child("Models"))
                    .children(models.into_iter().map(|(model, stats)| {
                        h_flex()
                            .justify_between()
                            .py_1p5()
                            .border_b_1()
                            .border_color(border)
                            .child(div().child(model.clone()))
                            .child(div().text_sm().text_color(muted).child(format!(
                                "{} requests · {} in · {} out",
                                stats.requests, stats.input_tokens, stats.output_tokens
                            )))
                    })),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_sm().text_color(cx.theme().danger).child(e)),
            )
    }
}
