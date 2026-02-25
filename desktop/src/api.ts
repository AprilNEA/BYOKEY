import { invoke } from "@tauri-apps/api/core";

// ── Types ────────────────────────────────────────────────────────────────────

export interface ProviderStatus {
  id: string;
  state: "valid" | "expired" | "not_authenticated";
}

export interface ProxyStatus {
  running: boolean;
  port: number;
}

export interface ProviderConfig {
  enabled: boolean;
  backend: string | null;
  fallback: string | null;
}

export interface AppConfig {
  host: string;
  port: number;
  providers: Record<string, ProviderConfig>;
}

export interface AccountInfo {
  account_id: string;
  label: string | null;
  is_active: boolean;
}

// ── API calls ────────────────────────────────────────────────────────────────

export function getProvidersStatus(): Promise<ProviderStatus[]> {
  return invoke<ProviderStatus[]>("get_providers_status");
}

export function getProxyStatus(): Promise<ProxyStatus> {
  return invoke<ProxyStatus>("get_proxy_status");
}

export function toggleProxy(): Promise<ProxyStatus> {
  return invoke<ProxyStatus>("toggle_proxy");
}

export function loginProvider(provider: string): Promise<void> {
  return invoke("login_provider", { provider });
}

export function logoutProvider(provider: string): Promise<void> {
  return invoke("logout_provider", { provider });
}

export function getConfig(): Promise<AppConfig> {
  return invoke<AppConfig>("get_config");
}

export function saveConfig(config: AppConfig): Promise<void> {
  return invoke("save_config", { config });
}

export function listAccounts(provider: string): Promise<AccountInfo[]> {
  return invoke<AccountInfo[]>("list_accounts", { provider });
}

export function switchAccount(provider: string, account: string): Promise<void> {
  return invoke("switch_account", { provider, account });
}

export function logoutAccount(provider: string, account: string): Promise<void> {
  return invoke("logout_account", { provider, account });
}
