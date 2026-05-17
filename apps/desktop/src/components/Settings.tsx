// Settings — settings panel. See spec section 6.7.

import { useEffect, useState } from "react";
import { tauri, type Settings as SettingsType } from "../lib/tauri";

const DEFAULTS: Record<string, string> = {
  proxy_port: "4200",
  session_timeout_minutes: "30",
  compression_enabled: "true",
  graph_injection_enabled: "true",
  graph_max_tokens: "500",
  openai_base_url: "https://api.openai.com/v1",
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

  useEffect(() => {
    void tauri.getSettings().then(setSettings);
    void tauri.getAppVersion().then(setAppVersion);
  }, []);

  const update = async (key: string, value: string) => {
    setSettings((s) => ({ ...s, [key]: value }));
    await tauri.updateSetting(key, value);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
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

        {/* Upstream URL */}
        <SettingRow label={LABELS.openai_base_url!}>
          <input
            type="text"
            value={settings.openai_base_url ?? "https://api.openai.com/v1"}
            onChange={(e) => update("openai_base_url", e.target.value)}
            className="rounded border border-border bg-background px-3 py-1.5 font-mono text-xs text-text-primary w-56"
            placeholder="https://api.openai.com/v1"
          />
        </SettingRow>
        <p className="-mt-3 text-xs text-text-secondary/60 ml-[1px]">
          Set to https://api.deepseek.com/v1 for DeepSeek, or any OpenAI-compatible endpoint.
        </p>
      </div>

      {/* Account info */}
      <div className="mt-8 rounded-lg border border-border bg-surface p-4">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
          Account
        </h3>
        <p className="mt-2 text-sm text-text-primary">
          Tier: <span className="font-medium capitalize">{settings.tier}</span>
        </p>
        <p className="text-sm text-text-primary">
          Sessions saved this month:{" "}
          <span className="font-medium">
            {settings.sessions_saved_this_month}
          </span>
        </p>
        {settings.tier === "free" && (
          <a
            href="https://sessiongraph.dev/upgrade"
            target="_blank"
            rel="noopener noreferrer"
            className="mt-2 inline-block text-sm text-accent hover:underline"
          >
            Upgrade to Pro →
          </a>
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
}: {
  on: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <button
      onClick={() => onChange(!on)}
      className={`relative h-6 w-11 rounded-full transition-colors ${
        on ? "bg-accent" : "bg-border"
      }`}
    >
      <span
        className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
          on ? "translate-x-5" : ""
        }`}
      />
    </button>
  );
}
