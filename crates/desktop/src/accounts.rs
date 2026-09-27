//! Stored accounts, and adding, switching and removing them.

use byokey_proto::byokey::accounts as acct;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Task, Window, div, px,
};

use byokey_proto::client::ManagementClient;

use crate::server::{LoginUpdate, Server};

/// An interactive login in flight. Dropping it abandons the login.
struct Login {
    provider: String,
    step: LoginUpdate,
    _task: Task<()>,
}

pub struct AccountsPage {
    providers: Vec<acct::ProviderAccounts>,
    login: Option<Login>,
    error: Option<SharedString>,
    api_key: Entity<InputState>,
}

impl AccountsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            providers: Vec::new(),
            login: None,
            error: None,
            api_key: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("API key")
            }),
        }
    }

    #[expect(
        clippy::unused_self,
        reason = "the receiver lets `Entity::update(cx, Self::refresh)` take it by path"
    )]
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let list = Server::call(cx, |c| async move { Ok(c.list_accounts().await?) });
        cx.spawn(async move |this, cx| {
            let result = list.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(list) => {
                        this.providers = list.providers;
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

    /// Run a write, then show its outcome and reload the list.
    fn write<F>(
        done: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        call: impl FnOnce(ManagementClient) -> F,
    ) where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let call = Server::call(cx, call);
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await;
            this.update_in(cx, |this, window, cx| {
                let note = match result {
                    Ok(()) => Notification::success(done),
                    Err(e) => Notification::error(format!("{e:#}")),
                };
                window.push_notification(note, cx);
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    fn start_login(&mut self, provider: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut updates = Server::login(
            cx,
            acct::LoginRequest {
                provider: provider.clone(),
                ..Default::default()
            },
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(update) = updates.recv().await {
                let keep_going = this
                    .update_in(cx, |this, window, cx| this.login_step(update, window, cx))
                    .unwrap_or(false);
                if !keep_going {
                    return;
                }
            }
        });
        self.login = Some(Login {
            provider,
            step: LoginUpdate::Exchanging,
            _task: task,
        });
        cx.notify();
    }

    /// Apply a login event; `false` once the login is over.
    fn login_step(
        &mut self,
        update: LoginUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let over = match &update {
            LoginUpdate::Visit { url, .. } => {
                cx.open_url(url);
                false
            }
            LoginUpdate::Exchanging => false,
            LoginUpdate::Done => {
                window.push_notification(Notification::success("Signed in"), cx);
                self.refresh(cx);
                true
            }
            LoginUpdate::Failed(e) => {
                window.push_notification(Notification::error(format!("Sign-in failed: {e}")), cx);
                true
            }
        };
        if over {
            self.login = None;
        } else if let Some(login) = &mut self.login {
            login.step = update;
        }
        cx.notify();
        !over
    }

    fn open_api_key_dialog(
        &mut self,
        provider: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.api_key.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
            dialog
                .w(px(420.))
                .title(format!("Add a {provider} API key"))
                .child(Input::new(&input))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Add")
                        .show_cancel(true),
                )
                .on_ok({
                    let (input, page, provider) = (input.clone(), page.clone(), provider.clone());
                    move |_, window, cx| {
                        let api_key = input.read(cx).value().to_string();
                        let provider = provider.clone();
                        page.update(cx, |_, cx| {
                            Self::write("API key added", window, cx, |c| async move {
                                c.add_api_key(acct::AddApiKeyRequest {
                                    provider,
                                    api_key,
                                    ..Default::default()
                                })
                                .await?;
                                Ok(())
                            });
                        });
                        true
                    }
                })
        });
    }

    fn confirm_remove(
        provider: String,
        account_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .w(px(420.))
                .title(format!("Remove {account_id}?"))
                .child(format!(
                    "BYOKEY forgets this {provider} account's credentials. Sign in again to get it back."
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Remove")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true),
                )
                .on_ok({
                    let (page, provider, account_id) =
                        (page.clone(), provider.clone(), account_id.clone());
                    move |_, window, cx| {
                        let (provider, account_id) = (provider.clone(), account_id.clone());
                        page.update(cx, |_, cx| {
                            Self::write("Removed account", window, cx, |c| async move {
                                Ok(c.remove_account(&provider, &account_id).await?)
                            });
                        });
                        true
                    }
                })
        });
    }

    fn render_login(login: &Login, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (message, code, url): (SharedString, _, _) = match &login.step {
            LoginUpdate::Visit { url, user_code } => (
                "Finish signing in in your browser.".into(),
                user_code.clone(),
                Some(url.clone()),
            ),
            _ => ("Waiting for the server…".into(), None, None),
        };
        v_flex()
            .p_3()
            .gap_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .bg(cx.theme().muted)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!("Signing in to {}", login.provider)),
            )
            .child(div().text_sm().child(message))
            .children(code.map(|code| {
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(code.clone()),
                    )
                    .child(
                        Button::new("copy-code")
                            .small()
                            .label("Copy code")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(code.clone()));
                            }),
                    )
            }))
            .child(
                h_flex()
                    .gap_2()
                    .children(url.map(|url| {
                        Button::new("open-url")
                            .small()
                            .label("Open browser again")
                            .on_click(move |_, _, cx| cx.open_url(&url))
                    }))
                    .child(
                        Button::new("cancel-login")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.login = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_provider(
        &self,
        provider: &acct::ProviderAccounts,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let id = provider.id.clone();
        let busy = self.login.is_some();
        let actions = h_flex()
            .gap_1()
            .child(
                Button::new(SharedString::from(format!("login-{id}")))
                    .small()
                    .primary()
                    .label("Sign in")
                    .disabled(busy)
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| this.start_login(id.clone(), window, cx)
                    })),
            )
            .children(accepts_api_key(&id).then(|| {
                Button::new(SharedString::from(format!("key-{id}")))
                    .small()
                    .label("Add API key")
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| this.open_api_key_dialog(id.clone(), window, cx)
                    }))
            }))
            .children((id == "claude").then(|| {
                Button::new("import-claude-code")
                    .small()
                    .label("Import Claude Code")
                    .on_click(cx.listener(|_, _, window, cx| {
                        Self::write(
                            "Imported the Claude Code login",
                            window,
                            cx,
                            |c| async move {
                                c.import_claude_code(acct::ImportClaudeCodeRequest::default())
                                    .await?;
                                Ok(())
                            },
                        );
                    }))
            }));

        let rows: Vec<_> = provider
            .accounts
            .iter()
            .map(|account| Self::render_account(&id, account, cx))
            .collect();
        let muted = cx.theme().muted_foreground;

        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(provider.display_name.clone()),
                    )
                    .child(actions),
            )
            .when(provider.accounts.is_empty(), |el| {
                el.child(div().text_sm().text_color(muted).child("No accounts"))
            })
            .children(rows)
    }

    fn render_account(
        provider: &str,
        account: &acct::AccountDetail,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let key = format!("{provider}-{}", account.account_id);
        let state = match account.token_state.as_known() {
            Some(acct::TokenState::TOKEN_STATE_VALID) => Tag::success().small().child("Valid"),
            Some(acct::TokenState::TOKEN_STATE_EXPIRED) => Tag::warning().small().child("Expired"),
            _ => Tag::danger().small().child("Invalid"),
        };
        let name = account.label.clone().map_or_else(
            || account.account_id.clone(),
            |label| format!("{label} ({})", account.account_id),
        );
        let target = (provider.to_owned(), account.account_id.clone());
        h_flex()
            .justify_between()
            .py_1p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(div().child(name))
                    .child(state)
                    .children(
                        account
                            .is_active
                            .then(|| Tag::primary().small().outline().child("Active")),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children((!account.is_active).then(|| {
                        let (provider, account_id) = target.clone();
                        Button::new(SharedString::from(format!("activate-{key}")))
                            .small()
                            .label("Use")
                            .on_click(cx.listener(move |_, _, window, cx| {
                                let (provider, account_id) = (provider.clone(), account_id.clone());
                                Self::write("Switched account", window, cx, |c| async move {
                                    Ok(c.activate_account(&provider, &account_id).await?)
                                });
                            }))
                    }))
                    .child({
                        let (provider, account_id) = target;
                        Button::new(SharedString::from(format!("remove-{key}")))
                            .small()
                            .danger()
                            .label("Remove")
                            .on_click(cx.listener(move |_, _, window, cx| {
                                Self::confirm_remove(
                                    provider.clone(),
                                    account_id.clone(),
                                    window,
                                    cx,
                                );
                            }))
                    }),
            )
    }
}

/// Copilot only signs in by device code; the others also take a raw key.
fn accepts_api_key(provider: &str) -> bool {
    provider != "copilot"
}

impl Render for AccountsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let login = self
            .login
            .as_ref()
            .map(|login| Self::render_login(login, cx));
        let mut providers = Vec::with_capacity(self.providers.len());
        for provider in &self.providers {
            providers.push(self.render_provider(provider, cx).into_any_element());
        }
        v_flex()
            .id("accounts")
            .size_full()
            .overflow_y_scroll()
            .p_5()
            .gap_5()
            .child(div().text_xl().child("Accounts"))
            .children(login)
            .children(providers)
            .children(
                self.error
                    .clone()
                    .map(|e| div().text_sm().text_color(cx.theme().danger).child(e)),
            )
    }
}
