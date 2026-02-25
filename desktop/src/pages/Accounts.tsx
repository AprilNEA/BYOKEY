import { useState, useEffect, useCallback } from "react";
import {
  getProvidersStatus,
  listAccounts,
  switchAccount,
  logoutAccount,
  type ProviderStatus,
  type AccountInfo,
} from "../api";

const PROVIDERS_WITH_AUTH = [
  "claude", "codex", "copilot", "gemini",
  "antigravity", "qwen", "kimi", "iflow",
];

interface ProviderAccounts {
  provider: string;
  accounts: AccountInfo[];
}

export function Accounts() {
  const [data, setData] = useState<ProviderAccounts[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const results = await Promise.all(
        PROVIDERS_WITH_AUTH.map(async (p) => {
          try {
            const accounts = await listAccounts(p);
            return { provider: p, accounts };
          } catch {
            return { provider: p, accounts: [] };
          }
        })
      );
      setData(results.filter((r) => r.accounts.length > 0));
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const handleSwitch = async (provider: string, accountId: string) => {
    setBusy(`${provider}:${accountId}`);
    try {
      await switchAccount(provider, accountId);
      await refresh();
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(null);
    }
  };

  const handleLogout = async (provider: string, accountId: string) => {
    setBusy(`${provider}:${accountId}`);
    try {
      await logoutAccount(provider, accountId);
      await refresh();
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(null);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-text-muted">
        Loading…
      </div>
    );
  }

  if (data.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-text-muted text-xs gap-1">
        <span>No accounts found</span>
        <span className="text-[11px]">Login from the Dashboard first</span>
      </div>
    );
  }

  return (
    <div className="overflow-y-auto py-2">
      {data.map(({ provider, accounts }) => (
        <div key={provider} className="mb-3">
          <h3 className="px-4 py-1 text-[11px] font-semibold text-text-muted uppercase tracking-wider">
            {provider}
          </h3>
          {accounts.map((a) => {
            const key = `${provider}:${a.account_id}`;
            return (
              <div
                key={key}
                className="flex items-center gap-2 px-4 py-1.5 hover:bg-bg-hover transition-colors"
              >
                <span className="flex-1 text-xs">
                  {a.label ?? a.account_id}
                  {a.is_active && (
                    <span className="ml-1.5 text-[10px] text-accent font-medium">
                      active
                    </span>
                  )}
                </span>
                {!a.is_active && (
                  <button
                    disabled={busy === key}
                    onClick={() => handleSwitch(provider, a.account_id)}
                    className="text-[11px] px-2 py-0.5 rounded border border-accent text-accent hover:bg-accent/10 transition-colors disabled:opacity-50"
                  >
                    Switch
                  </button>
                )}
                <button
                  disabled={busy === key}
                  onClick={() => handleLogout(provider, a.account_id)}
                  className="text-[11px] px-2 py-0.5 rounded border border-border text-text-muted hover:border-danger hover:text-danger transition-colors disabled:opacity-50"
                >
                  Remove
                </button>
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
