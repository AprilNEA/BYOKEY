import { useState, useEffect, useCallback } from "react";
import {
  getProvidersStatus,
  toggleProxy,
  loginProvider,
  logoutProvider,
  type ProviderStatus,
  type ProxyStatus,
} from "../api";

const STATE_LABELS: Record<string, string> = {
  valid: "Authenticated",
  expired: "Expired",
  not_authenticated: "Not logged in",
};

interface Props {
  proxyStatus: ProxyStatus;
  onProxyChange: (s: ProxyStatus) => void;
}

export function Dashboard({ proxyStatus, onProxyChange }: Props) {
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyProvider, setBusyProvider] = useState<string | null>(null);
  const [togglingProxy, setTogglingProxy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const data = await getProvidersStatus();
      setProviders(data);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, [refresh]);

  const handleToggleProxy = async () => {
    setTogglingProxy(true);
    try {
      const status = await toggleProxy();
      onProxyChange(status);
    } catch (e) {
      console.error(e);
    } finally {
      setTogglingProxy(false);
    }
  };

  const handleProviderAction = async (provider: string, isLoggedIn: boolean) => {
    setBusyProvider(provider);
    try {
      if (isLoggedIn) {
        await logoutProvider(provider);
      } else {
        await loginProvider(provider);
      }
      await refresh();
    } catch (e) {
      console.error(e);
    } finally {
      setBusyProvider(null);
    }
  };

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full text-text-muted">
        Loading…
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      {/* Provider list */}
      <div className="flex-1 py-2">
        {providers.map((p) => {
          const isLoggedIn = p.state === "valid" || p.state === "expired";
          const stateColor =
            p.state === "valid"
              ? "text-success"
              : p.state === "expired"
                ? "text-warning"
                : "text-danger";
          const stateIcon =
            p.state === "valid" ? "✓" : p.state === "expired" ? "○" : "✗";

          return (
            <div
              key={p.id}
              className="flex items-center gap-2.5 px-4 py-2 hover:bg-bg-hover transition-colors"
            >
              <span className={`w-4.5 text-center text-[13px] shrink-0 ${stateColor}`}>
                {stateIcon}
              </span>
              <span className="flex-1 font-medium capitalize">{p.id}</span>
              <span className="text-[11px] text-text-muted mr-1">
                {STATE_LABELS[p.state] ?? p.state}
              </span>
              <button
                disabled={busyProvider === p.id}
                onClick={() => handleProviderAction(p.id, isLoggedIn)}
                className={`shrink-0 text-[11px] px-2.5 py-0.5 rounded-md border transition-colors disabled:opacity-50 ${
                  isLoggedIn
                    ? "border-border text-text-muted hover:border-text-muted hover:text-text hover:bg-bg-hover"
                    : "border-accent text-accent hover:bg-accent/10 hover:text-accent-hover"
                }`}
                style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
              >
                {isLoggedIn ? "Logout" : "Login"}
              </button>
            </div>
          );
        })}
      </div>

      {/* Footer with proxy toggle */}
      <div className="flex items-center justify-between px-4 py-3 border-t border-border shrink-0">
        <button
          disabled={togglingProxy}
          onClick={handleToggleProxy}
          className={`px-4 py-1.5 text-[13px] font-medium rounded-md text-white transition-colors disabled:opacity-50 ${
            proxyStatus.running
              ? "bg-danger hover:bg-red-500"
              : "bg-accent hover:bg-accent-hover"
          }`}
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        >
          {proxyStatus.running ? "Stop Proxy" : "Start Proxy"}
        </button>
        <span className="text-xs text-text-muted tabular-nums">
          port: {proxyStatus.port}
        </span>
      </div>
    </div>
  );
}
