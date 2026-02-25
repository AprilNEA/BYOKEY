import { useState, useEffect } from "react";
import { Switch } from "@base-ui/react/switch";
import { getConfig, saveConfig, type AppConfig, type ProviderConfig } from "../api";

const ALL_PROVIDERS = [
  "claude", "codex", "copilot", "gemini", "kiro",
  "antigravity", "qwen", "kimi", "iflow",
];

export function Settings() {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);

  useEffect(() => {
    getConfig()
      .then(setConfig)
      .catch((e) => setError(String(e)));
  }, []);

  const update = (patch: Partial<AppConfig>) => {
    if (!config) return;
    setConfig({ ...config, ...patch });
    setDirty(true);
  };

  const updateProvider = (id: string, patch: Partial<ProviderConfig>) => {
    if (!config) return;
    const prev = config.providers[id] ?? { enabled: true, backend: null, fallback: null };
    setConfig({
      ...config,
      providers: { ...config.providers, [id]: { ...prev, ...patch } },
    });
    setDirty(true);
  };

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setError(null);
    try {
      await saveConfig(config);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  if (error && !config) {
    return (
      <div className="flex items-center justify-center h-full text-danger text-xs px-4 text-center">
        {error}
      </div>
    );
  }

  if (!config) {
    return (
      <div className="flex items-center justify-center h-full text-text-muted">
        Loading…
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 overflow-y-auto px-4 py-3 space-y-4">
        {/* Server section */}
        <section>
          <h3 className="text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
            Server
          </h3>
          <div className="space-y-2">
            <label className="flex items-center justify-between">
              <span className="text-xs">Host</span>
              <input
                type="text"
                value={config.host}
                onChange={(e) => update({ host: e.target.value })}
                className="w-36 bg-bg-elevated border border-border rounded px-2 py-1 text-xs text-text focus:outline-none focus:border-accent"
              />
            </label>
            <label className="flex items-center justify-between">
              <span className="text-xs">Port</span>
              <input
                type="number"
                value={config.port}
                onChange={(e) => update({ port: Number(e.target.value) || 8018 })}
                className="w-36 bg-bg-elevated border border-border rounded px-2 py-1 text-xs text-text focus:outline-none focus:border-accent"
              />
            </label>
          </div>
        </section>

        {/* Providers section */}
        <section>
          <h3 className="text-xs font-semibold text-text-muted uppercase tracking-wider mb-2">
            Providers
          </h3>
          <div className="space-y-1">
            {ALL_PROVIDERS.map((id) => {
              const pc = config.providers[id];
              const enabled = pc?.enabled ?? true;
              return (
                <div
                  key={id}
                  className="flex items-center justify-between py-1.5"
                >
                  <span className="text-xs capitalize">{id}</span>
                  <Switch.Root
                    checked={enabled}
                    onCheckedChange={(checked) =>
                      updateProvider(id, { enabled: checked })
                    }
                    className="relative flex h-4.5 w-8 rounded-full bg-border p-px transition-colors data-[checked]:bg-accent"
                  >
                    <Switch.Thumb className="h-3.5 w-3.5 rounded-full bg-white transition-transform data-[checked]:translate-x-3.5" />
                  </Switch.Root>
                </div>
              );
            })}
          </div>
        </section>
      </div>

      {/* Save footer */}
      {dirty && (
        <div className="flex items-center justify-between px-4 py-3 border-t border-border shrink-0">
          {error && <span className="text-[11px] text-danger truncate mr-2">{error}</span>}
          <div className="ml-auto">
            <button
              disabled={saving}
              onClick={handleSave}
              className="px-4 py-1.5 text-[13px] font-medium rounded-md text-white bg-accent hover:bg-accent-hover transition-colors disabled:opacity-50"
            >
              {saving ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
