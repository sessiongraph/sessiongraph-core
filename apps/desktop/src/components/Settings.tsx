// Settings — settings panel. See spec section 6.7.

import { useEffect, useRef, useState } from "react";
import { tauri, type Settings as SettingsType, type SystemProxyStatus } from "../lib/tauri";

const DEFAULTS: Record<string, string> = {
  proxy_port: "4200",
  session_timeout_minutes: "30",
  compression_enabled: "true",
  graph_injection_enabled: "true",
  graph_max_tokens: "500",
  anthropic_base_url: "https://api.anthropic.com",
  openai_base_url: "https://api.openai.com",
  tier: "free",
  sessions_saved_this_month: "0",
  onboarding_complete: "false",
};

const LABELS: Record<string, string> = {
  proxy_port: "Proxy port",
  session_timeout_minutes: "Session timeout (minutes)",
  compression_enabled: "Compression",
  graph_injection_enabled: "Graph injection",
  graph_max_tokens: "Max graph tokens",
  anthropic_base_url: "Anthropic upstream URL",
  openai_base_url: "OpenAI-compatible upstream URL",
};

interface Props {
  onClose: () => void;
}

export default function Settings({ onClose }: Props) {
  const [settings, setSettings] = useState<SettingsType>(DEFAULTS);
  const [saved, setSaved] = useState(false);
  const [appVersion, setAppVersion] = useState("…");
  const [deleting, setDeleting] = useState(false);
  const [systemProxy, setSystemProxy] = useState<SystemProxyStatus | null>(null);
  const [proxyToggling, setProxyToggling] = useState(false);
  const [licenseInput, setLicenseInput] = useState("");
  const [licenseError, setLicenseError] = useState<string | null>(null);
  const [licenseActivating, setLicenseActivating] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingRef = useRef<Record<string, string>>({});

  useEffect(() => {
    void tauri.getSettings().then(setSettings);
    void tauri.getAppVersion().then(setAppVersion);
    void tauri.getSystemProxyStatus().then(setSystemProxy);
  }, []);

  const handleActivateLicense = async () => {
    const key = licenseInput.trim();
    if (!key) return;
    setLicenseActivating(true);
    setLicenseError(null);
    try {
      const status = await tauri.activateLicense(key);
      setSettings((s) => ({ ...s, tier: status.tier }));
      setLicenseInput("");
      // Trigger immediate sync so the web dashboard updates right away.
      void tauri.syncUsageNow().catch(() => {});
    } catch (e) {
      setLicenseError(e instanceof Error ? e.message : String(e));
    } finally {
      setLicenseActivating(false);
    }
  };

  const update = (key: string, value: string) => {
    setSettings((s) => ({ ...s, [key]: value }));
    pendingRef.current[key] = value;
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      const pending = { ...pendingRef.current };
      pendingRef.current = {};
      void Promise.all(
        Object.entries(pending).map(([k, v]) => tauri.updateSetting(k, v)),
      ).then(() => {
        setSaved(true);
        setTimeout(() => setSaved(false), 1500);
      });
    }, 300);
  };

  const handleToggleProxy = async () => {
    if (!systemProxy) return;
    setProxyToggling(true);
    try {
      await tauri.setSystemProxy(!systemProxy.enabled);
      setSystemProxy({ ...systemProxy, enabled: !systemProxy.enabled });
    } catch (e) {
      console.error("Proxy toggle failed:", e);
    }
    setProxyToggling(false);
  };

  const handleReset = async () => {
    if (!confirm("Delete all session data? This cannot be undone.")) return;
    setDeleting(true);
    try {
      await tauri.deleteAllData();
    } catch (e) {
      console.error("Delete failed:", e);
    }
    setDeleting(false);
  };

  return (
    <main className="mx-auto max-w-lg px-8 py-10">
      <div className="flex items-center justify-between border-b border-border pb-4">
        <h1 className="text-lg font-semibold">Settings</h1>
        <button
          onClick={onClose}
          className="text-sm text-text-secondary hover:text-text-primary"
        >
          ← Dashboard
        </button>
      </div>

      <div className="mt-6 space-y-5">
        {/* Proxy port */}
        <SettingRow label={LABELS.proxy_port!}>
          <input
            type="number"
            value={settings.proxy_port ?? "4200"}
            onChange={(e) => update("proxy_port", e.target.value)}
            className="w-24 rounded border border-border bg-background px-3 py-1.5 font-mono text-sm text-text-primary"
            min={1024}
            max={65535}
          />
        </SettingRow>

        {/* Session timeout */}
        <SettingRow label={LABELS.session_timeout_minutes!}>
          <input
            type="number"
            value={settings.session_timeout_minutes ?? "30"}
            onChange={(e) => update("session_timeout_minutes", e.target.value)}
            className="w-20 rounded border border-border bg-background px-3 py-1.5 font-mono text-sm text-text-primary"
            min={5}
            max={240}
          />
        </SettingRow>

        {/* Compression */}
        <SettingRow label={LABELS.compression_enabled!}>
          <Toggle
            on={settings.compression_enabled !== "false"}
            onChange={(on) => update("compression_enabled", on ? "true" : "false")}
          />
        </SettingRow>

        {/* Graph injection */}
        <SettingRow label={LABELS.graph_injection_enabled!}>
          <Toggle
            on={settings.graph_injection_enabled !== "false"}
            onChange={(on) =>
              update("graph_injection_enabled", on ? "true" : "false")
            }
          />
        </SettingRow>

        {/* Max graph tokens */}
        <SettingRow label={LABELS.graph_max_tokens!}>
          <input
            type="range"
            min={100}
            max={1000}
            step={50}
            value={settings.graph_max_tokens ?? "500"}
            onChange={(e) => update("graph_max_tokens", e.target.value)}
            className="w-40"
          />
          <span className="ml-2 font-mono text-xs text-text-secondary">
            {settings.graph_max_tokens ?? "500"} tokens
          </span>
        </SettingRow>

        {/* Anthropic URL */}
        <SettingRow label={LABELS.anthropic_base_url!}>
          <input
            type="text"
            value={settings.anthropic_base_url ?? "https://api.anthropic.com"}
            onChange={(e) => update("anthropic_base_url", e.target.value)}
            className="rounded border border-border bg-background px-3 py-1.5 font-mono text-xs text-text-primary w-56"
            placeholder="https://api.anthropic.com"
          />
        </SettingRow>

        {/* OpenAI-compatible URL */}
        <SettingRow label={LABELS.openai_base_url!}>
          <input
            type="text"
            value={settings.openai_base_url ?? "https://api.openai.com"}
            onChange={(e) => update("openai_base_url", e.target.value)}
            className="rounded border border-border bg-background px-3 py-1.5 font-mono text-xs text-text-primary w-56"
            placeholder="https://api.openai.com"
          />
        </SettingRow>
        <p className="-mt-3 text-xs text-text-secondary/60 ml-[1px]">
          Set to a provider base URL (e.g. https://api.deepseek.com) for any OpenAI-compatible endpoint. Path is appended automatically.
        </p>

        {/* System proxy (PAC) */}
        <SettingRow label="Auto-route AI traffic">
          <div className="flex items-center gap-2">
            {systemProxy === null ? (
              <span className="text-xs text-text-secondary">…</span>
            ) : (
              <>
                <Toggle
                  on={systemProxy.enabled}
                  onChange={handleToggleProxy}
                  disabled={proxyToggling}
                />
                <span className="text-xs text-text-secondary/60">
                  {proxyToggling
                    ? "…"
                    : systemProxy.enabled
                      ? "On"
                      : "Off"}
                </span>
              </>
            )}
          </div>
        </SettingRow>
        <p className="-mt-3 text-xs text-text-secondary/60 ml-[1px]">
          When enabled, AI API traffic automatically routes through SessionGraph using system proxy settings. Apps fall back to direct connection when SessionGraph is closed.
        </p>
      </div>

      {/* License */}
      <div className="mt-8 rounded-lg border border-border bg-surface p-4 space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
          License
        </h3>
        <div className="flex items-center justify-between">
          <span className="text-sm text-text-primary">Plan</span>
          <span className={`text-sm font-semibold capitalize ${settings.tier !== "free" ? "text-accent" : "text-text-secondary"}`}>
            {settings.tier}
          </span>
        </div>
        <div className="flex items-center justify-between">
          <span className="text-sm text-text-primary">Sessions saved this month</span>
          <span className="text-sm font-medium text-text-primary">{settings.sessions_saved_this_month}</span>
        </div>

        {settings.tier === "free" ? (
          <div className="space-y-2 pt-1">
            <p className="text-xs text-text-secondary/70">
              Have a license key? Paste it below to activate Pro.
            </p>
            <textarea
              value={licenseInput}
              onChange={(e) => setLicenseInput(e.target.value)}
              placeholder="Paste your license key (JWT)…"
              rows={3}
              className="w-full rounded border border-border bg-background px-3 py-2 font-mono text-xs text-text-primary resize-none focus:outline-none focus:border-accent"
            />
            {licenseError && (
              <p className="text-xs text-red-400">{licenseError}</p>
            )}
            <div className="flex items-center gap-3">
              <button
                onClick={handleActivateLicense}
                disabled={licenseActivating || !licenseInput.trim()}
                className="rounded bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity disabled:opacity-50"
              >
                {licenseActivating ? "Activating…" : "Activate License"}
              </button>
              <a
                href="https://www.sessiongraph.dev/#pricing"
                target="_blank"
                rel="noopener noreferrer"
                className="text-xs text-accent hover:underline"
              >
                Get a license →
              </a>
            </div>
          </div>
        ) : (
          <p className="text-xs text-text-secondary/60">
            License active. To update or revoke, visit{" "}
            <a href="https://www.sessiongraph.dev/dashboard/billing" target="_blank" rel="noopener noreferrer" className="text-accent hover:underline">
              sessiongraph.dev/dashboard/billing
            </a>.
          </p>
        )}
      </div>

      {/* Data management */}
      <div className="mt-4 rounded-lg border border-border bg-surface p-4">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
          Data
        </h3>
        <p className="mt-1 text-xs text-text-secondary">
          All data is stored locally at <code>~/.sessiongraph/</code>.
        </p>
        <button
          onClick={handleReset}
          disabled={deleting}
          className="mt-3 rounded border border-border px-3 py-1.5 text-xs text-text-secondary transition-colors hover:border-red-400 hover:text-red-400 disabled:opacity-50"
        >
          {deleting ? "Deleting…" : "Delete all data"}
        </button>
      </div>

      {/* About */}
      <div className="mt-4 rounded-lg border border-border bg-surface p-4">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
          About
        </h3>
        <p className="mt-1 text-xs text-text-secondary">
          SessionGraph v{appVersion}
        </p>
      </div>

      {saved && (
        <p className="mt-4 text-center text-xs text-success">Settings saved</p>
      )}
    </main>
  );
}

// ── Sub-components ─────────────────────────────────────────────────────────

function SettingRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-text-primary">{label}</span>
      <div className="flex items-center">{children}</div>
    </div>
  );
}

function Toggle({
  on,
  onChange,
  disabled,
}: {
  on: boolean;
  onChange: (on: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={() => onChange(!on)}
      disabled={disabled}
      className={`relative h-6 w-11 rounded-full transition-colors ${
        on ? "bg-accent" : "bg-border"
      } ${disabled ? "opacity-50" : ""}`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
          on ? "translate-x-5" : ""
        }`}
      />
    </button>
  );
}
