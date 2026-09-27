//! The management API, reached from GPUI.
//!
//! The `ConnectRPC` client runs on hyper and needs a Tokio runtime, which
//! GPUI's executors aren't. Calls run on a small runtime owned here; their
//! `JoinHandle`s are plain futures, so GPUI tasks can await them.

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context as _, Result, bail};
use byokey_proto::byokey::accounts as acct;
use byokey_proto::client::ManagementClient;
use gpui_kit::{App, Global};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use acct::__buffa::oneof::login_event::Event as LoginEvent;

const DEFAULT_URL: &str = "http://127.0.0.1:8018";

/// Where the server listens and a runtime to talk to it on.
pub struct Server {
    client: ManagementClient,
    runtime: Runtime,
}

impl Global for Server {}

impl Server {
    /// Connect to the endpoint in `BYOKEY_MANAGEMENT_URL`, or the default
    /// local server.
    ///
    /// # Errors
    ///
    /// An endpoint that isn't a local `http://host:port` URL, or a runtime
    /// that can't start.
    pub fn from_env() -> Result<Self> {
        let endpoint =
            std::env::var("BYOKEY_MANAGEMENT_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
        let uri: http::Uri = endpoint
            .parse()
            .with_context(|| format!("invalid management API URL {endpoint}"))?;
        if uri.scheme_str() != Some("http") || uri.authority().is_none() {
            bail!("management API URL must be http://host:port, got {endpoint}");
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("byokey-rpc")
            .enable_all()
            .build()
            .context("start the RPC runtime")?;
        Ok(Self {
            client: ManagementClient::local_http(uri),
            runtime,
        })
    }

    /// Run one call against the server. The call is under way on return;
    /// the future only waits for its result, so it borrows nothing.
    pub fn call<F, T>(
        cx: &App,
        f: impl FnOnce(ManagementClient) -> F,
    ) -> Pin<Box<dyn Future<Output = Result<T>> + Send>>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let server = cx.global::<Self>();
        let handle = server.runtime.spawn(f(server.client.clone()));
        Box::pin(async move { handle.await.context("RPC task panicked")? })
    }

    /// Start a login and receive its events. Dropping the receiver abandons
    /// the login on the server.
    pub fn login(cx: &App, request: acct::LoginRequest) -> mpsc::UnboundedReceiver<LoginUpdate> {
        let server = cx.global::<Self>();
        let client = server.client.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        server.runtime.spawn(async move {
            tokio::select! {
                () = forward_login(&client, request, &tx) => {}
                () = tx.closed() => {}
            }
        });
        rx
    }
}

/// One step of a login, as the UI shows it.
#[derive(Debug, Clone)]
pub enum LoginUpdate {
    Visit {
        url: String,
        user_code: Option<String>,
    },
    Exchanging,
    Done,
    Failed(String),
}

async fn forward_login(
    client: &ManagementClient,
    request: acct::LoginRequest,
    tx: &mpsc::UnboundedSender<LoginUpdate>,
) {
    let mut stream = match client.login(request).await {
        Ok(stream) => stream,
        Err(e) => {
            let _ = tx.send(LoginUpdate::Failed(e.to_string()));
            return;
        }
    };
    loop {
        let update = match stream.message().await {
            Ok(Some(message)) => match message.to_owned_message().event {
                Some(LoginEvent::Visit(visit)) => LoginUpdate::Visit {
                    url: visit.url,
                    user_code: visit.user_code,
                },
                Some(LoginEvent::Exchanging(_)) => LoginUpdate::Exchanging,
                Some(LoginEvent::Done(_)) => LoginUpdate::Done,
                Some(LoginEvent::Failed(failed)) => LoginUpdate::Failed(failed.error),
                None => continue,
            },
            Ok(None) => LoginUpdate::Failed("the server ended the login without a result".into()),
            Err(e) => LoginUpdate::Failed(e.to_string()),
        };
        let last = matches!(update, LoginUpdate::Done | LoginUpdate::Failed(_));
        if tx.send(update).is_err() || last {
            return;
        }
    }
}
