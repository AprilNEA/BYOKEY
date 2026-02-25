import { HashRouter, Routes, Route, NavLink } from "react-router-dom";
import { useState, useEffect, useCallback } from "react";
import { Dashboard } from "./pages/Dashboard";
import { Settings } from "./pages/Settings";
import { Accounts } from "./pages/Accounts";
import { getProxyStatus, type ProxyStatus } from "./api";

export function App() {
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus>({
    running: false,
    port: 8018,
  });

  const refreshProxy = useCallback(async () => {
    try {
      const status = await getProxyStatus();
      setProxyStatus(status);
    } catch (e) {
      console.error("Failed to fetch proxy status:", e);
    }
  }, []);

  useEffect(() => {
    refreshProxy();
    const timer = setInterval(refreshProxy, 5000);
    return () => clearInterval(timer);
  }, [refreshProxy]);

  useEffect(() => {
    const onVisChange = () => {
      if (!document.hidden) refreshProxy();
    };
    document.addEventListener("visibilitychange", onVisChange);
    return () => document.removeEventListener("visibilitychange", onVisChange);
  }, [refreshProxy]);

  return (
    <HashRouter>
      <div className="flex flex-col h-full">
        {/* Header — draggable */}
        <header
          className="flex items-center justify-between px-4 py-3 border-b border-border shrink-0"
          style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        >
          <span className="text-sm font-semibold tracking-wide">BYOKEY</span>
          <span className="flex items-center gap-1.5 text-xs text-text-muted">
            <span
              className={`w-2 h-2 rounded-full transition-colors ${
                proxyStatus.running
                  ? "bg-success shadow-[0_0_6px_rgba(74,222,128,0.4)]"
                  : "bg-text-muted"
              }`}
            />
            <span>{proxyStatus.running ? "Running" : "Stopped"}</span>
          </span>
        </header>

        {/* Main content */}
        <main className="flex-1 overflow-y-auto">
          <Routes>
            <Route
              path="/"
              element={
                <Dashboard
                  proxyStatus={proxyStatus}
                  onProxyChange={setProxyStatus}
                />
              }
            />
            <Route path="/settings" element={<Settings />} />
            <Route path="/accounts" element={<Accounts />} />
          </Routes>
        </main>

        {/* Bottom tab bar */}
        <nav className="flex items-center border-t border-border shrink-0">
          <TabLink to="/" label="Dashboard" />
          <TabLink to="/settings" label="Settings" />
          <TabLink to="/accounts" label="Accounts" />
        </nav>
      </div>
    </HashRouter>
  );
}

function TabLink({ to, label }: { to: string; label: string }) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) =>
        `flex-1 text-center py-2.5 text-xs font-medium transition-colors ${
          isActive ? "text-accent" : "text-text-muted hover:text-text"
        }`
      }
      style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
    >
      {label}
    </NavLink>
  );
}
