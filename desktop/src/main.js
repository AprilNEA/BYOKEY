const { invoke } = window.__TAURI__.core;

const $ = (sel) => document.querySelector(sel);

const STATE_LABELS = {
  valid: "authenticated",
  expired: "expired",
  refreshing: "refreshing",
  not_authenticated: "not logged in",
  invalid: "invalid",
};

const STATE_ICONS = {
  valid: "✓",
  expired: "○",
  refreshing: "↻",
  not_authenticated: "✗",
  invalid: "✗",
};

let proxyRunning = false;
let pollTimer = null;

// --- Rendering ---

function renderProviders(providers) {
  const list = $("#provider-list");
  if (!providers.length) {
    list.innerHTML = '<div class="loading">No providers configured</div>';
    return;
  }

  list.innerHTML = providers
    .map((p) => {
      const stateClass = p.state === "not_authenticated" ? "not-authenticated" : p.state;
      const icon = STATE_ICONS[p.state] ?? "?";
      const label = STATE_LABELS[p.state] ?? p.state;
      const isLoggedIn = p.state === "valid" || p.state === "expired" || p.state === "refreshing";
      const actionLabel = isLoggedIn ? "Logout" : "Login";
      const actionClass = isLoggedIn ? "logout" : "login";

      return `
        <div class="provider-row" data-id="${p.id}">
          <span class="provider-icon ${stateClass}">${icon}</span>
          <span class="provider-name">${p.id}</span>
          <span class="provider-state">${label}</span>
          <button class="provider-action ${actionClass}"
                  data-provider="${p.id}"
                  data-action="${actionLabel.toLowerCase()}">${actionLabel}</button>
        </div>`;
    })
    .join("");
}

function updateProxyUI(status) {
  proxyRunning = status.running;
  const dot = $("#status-dot");
  const text = $("#status-text");
  const btn = $("#toggle-proxy-btn");
  const port = $("#port-label");

  dot.classList.toggle("running", status.running);
  text.textContent = status.running ? "Running" : "Stopped";
  btn.textContent = status.running ? "Stop Proxy" : "Start Proxy";
  btn.classList.toggle("running", status.running);
  port.textContent = `port: ${status.port}`;
}

// --- Data fetching ---

async function fetchProviders() {
  try {
    const providers = await invoke("get_providers_status");
    renderProviders(providers);
  } catch (e) {
    console.error("Failed to fetch providers:", e);
  }
}

async function fetchProxyStatus() {
  try {
    const status = await invoke("get_proxy_status");
    updateProxyUI(status);
  } catch (e) {
    console.error("Failed to fetch proxy status:", e);
  }
}

async function refresh() {
  await Promise.all([fetchProviders(), fetchProxyStatus()]);
}

// --- Actions ---

async function handleToggleProxy() {
  const btn = $("#toggle-proxy-btn");
  btn.disabled = true;
  try {
    const status = await invoke("toggle_proxy");
    updateProxyUI(status);
    await fetchProviders();
  } catch (e) {
    console.error("Failed to toggle proxy:", e);
  } finally {
    btn.disabled = false;
  }
}

async function handleProviderAction(provider, action) {
  try {
    if (action === "login") {
      await invoke("login_provider", { provider });
    } else {
      await invoke("logout_provider", { provider });
    }
    await fetchProviders();
  } catch (e) {
    console.error(`Failed to ${action} ${provider}:`, e);
  }
}

// --- Event listeners ---

$("#toggle-proxy-btn").addEventListener("click", handleToggleProxy);

$("#provider-list").addEventListener("click", (e) => {
  const btn = e.target.closest(".provider-action");
  if (!btn) return;
  const provider = btn.dataset.provider;
  const action = btn.dataset.action;
  btn.disabled = true;
  handleProviderAction(provider, action).finally(() => {
    btn.disabled = false;
  });
});

// --- Init ---

document.addEventListener("DOMContentLoaded", () => {
  refresh();
  pollTimer = setInterval(refresh, 5000);
});

document.addEventListener("visibilitychange", () => {
  if (document.hidden) {
    clearInterval(pollTimer);
    pollTimer = null;
  } else {
    refresh();
    pollTimer = setInterval(refresh, 5000);
  }
});
