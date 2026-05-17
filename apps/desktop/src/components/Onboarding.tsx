// Onboarding — 4-step setup wizard. See spec section 6.6.

import { useState, useEffect } from "react";
import { tauri, type HealthStatus, type VenvStatus, type ProxyStatus } from "../lib/tauri";

type Step = 1 | 2 | 3 | 4;

interface Props {
  onComplete: () => void;
}

export default function Onboarding({ onComplete }: Props) {
  const [step, setStep] = useState<Step>(1);
  const [health, setHealth] = useState<HealthStatus | null>(null);
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);
  const [, setVenvStatus] = useState<VenvStatus | null>(null);
  const [venvSetupDone, setVenvSetupDone] = useState(false);
  const [checking, setChecking] = useState(false);
  const [profileInstalled, setProfileInstalled] = useState(false);
  const [profileInstalling, setProfileInstalling] = useState(false);
  const [profileResult, setProfileResult] = useState<string | null>(null);

  useEffect(() => {
    void tauri.getProxyStatus().then(setProxyStatus);
    void tauri.getCliProfileStatus().then((s) => setProfileInstalled(s.installed));
    void tauri.checkVenvStatus().then(setVenvStatus);
  }, []);

  const handleInstallProfile = async () => {
    setProfileInstalling(true);
    setProfileResult(null);
    try {
      const path = await tauri.addCliProfile();
      setProfileInstalled(true);
      setProfileResult(`Added to ${path}`);
    } catch (e) {
      setProfileResult(String(e));
    }
    setProfileInstalling(false);
  };

  const handleCheckHealth = async () => {
    setChecking(true);
    try {
      const h = await tauri.checkProxyHealth();
      setHealth(h);
      if (h.status === "healthy") {
        setTimeout(() => setStep(4), 800);
      }
    } catch {
      setHealth({ status: "unhealthy", proxy_version: "", uptime_seconds: 0 });
    }
    setChecking(false);
  };

  // Set up venv in background when onboarding completes
  const handleComplete = async () => {
    // Check venv status - if not ready, set it up in background
    const venv = await tauri.checkVenvStatus();
    if (!venv.ready) {
      setVenvSetupDone(true);
      tauri
        .setupVenv()
        .then(() => {
          console.log("Venv setup complete");
        })
        .catch((e) => {
          console.warn("Venv setup failed (non-fatal):", e);
        });
    }
    onComplete();
  };

  return (
    <main className="mx-auto max-w-lg px-8 py-20">
      {/* ── Step 1: Welcome ──────────────────────────── */}
      {step === 1 && (
        <div className="text-center">
          <h1 className="text-2xl font-semibold tracking-tight">
            Welcome to SessionGraph
          </h1>
          <p className="mt-4 text-text-secondary leading-relaxed">
            SessionGraph saves your AI coding context between sessions.
            Your AI remembers where you left off — automatically, invisibly,
            without changing how you work.
          </p>
          <div className="mt-8 flex justify-center gap-3">
            <button
              onClick={() => setStep(2)}
              className="rounded-lg bg-accent px-6 py-2.5 text-sm font-medium text-white transition-colors hover:bg-accent/90"
            >
              Get Started
            </button>
            <button
              onClick={onComplete}
              className="rounded-lg border border-border px-6 py-2.5 text-sm text-text-secondary transition-colors hover:bg-surface"
            >
              Skip for now
            </button>
          </div>
        </div>
      )}

      {/* ── Step 2: Setup Proxy ─────────────────────── */}
      {step === 2 && (
        <div>
          <h2 className="text-xl font-semibold">Set up the proxy</h2>
          <p className="mt-3 text-sm text-text-secondary">
            SessionGraph works by routing your AI API calls through a local
            proxy. Your API keys stay on your machine — nothing leaves your
            computer.
          </p>
          <p className="mt-4 text-sm text-text-secondary">
            One click to add auto-detection to your shell profile. CLI tools
            will use the proxy when running, or connect directly when closed.
          </p>
          <div className="mt-6 flex flex-col items-center gap-3">
            {profileInstalled ? (
              <p className="text-sm text-success">✓ Auto-detection is installed</p>
            ) : (
              <button
                onClick={handleInstallProfile}
                disabled={profileInstalling}
                className="rounded-lg bg-accent px-8 py-3 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
              >
                {profileInstalling ? "Installing…" : "Install auto-detection"}
              </button>
            )}
            {profileResult && (
              <p className={`text-xs ${profileResult.startsWith("Added") ? "text-success" : "text-amber-400"}`}>
                {profileResult}
              </p>
            )}
          </div>
          <div className="mt-6 flex justify-center gap-3">
            <button
              onClick={() => setStep(3)}
              className="rounded-lg bg-accent/20 px-6 py-2 text-sm text-text-primary transition-colors hover:bg-accent/30"
            >
              {profileInstalled ? "Next →" : "Skip for now"}
            </button>
          </div>
          <button
            onClick={() => setStep(1)}
            className="mt-4 block text-xs text-text-secondary hover:text-text-primary mx-auto"
          >
            ← Back
          </button>
        </div>
      )}

      {/* ── Step 3: Verify ──────────────────────────── */}
      {step === 3 && (
        <div className="text-center">
          <h2 className="text-xl font-semibold">Verifying proxy</h2>
          <p className="mt-3 text-sm text-text-secondary">
            Checking that the proxy server is running and reachable.
          </p>
          <div className="mt-8">
            {checking ? (
              <p className="text-text-secondary">Checking…</p>
            ) : health ? (
              health.status === "healthy" ? (
                <div>
                  <p className="text-3xl text-success">✓</p>
                  <p className="mt-2 text-sm text-success">
                    Proxy is running on port {proxyStatus?.port ?? 4200}
                  </p>
                  <p className="mt-1 text-xs text-text-secondary">
                    v{health.proxy_version} · uptime {health.uptime_seconds}s
                  </p>
                </div>
              ) : (
                <div>
                  <p className="text-3xl text-text-secondary">✗</p>
                  <p className="mt-2 text-sm text-text-secondary">
                    Proxy is not reachable. Make sure the app is running and
                    try again.
                  </p>
                </div>
              )
            ) : null}
          </div>
          <button
            onClick={handleCheckHealth}
            disabled={checking}
            className="mt-6 rounded-lg bg-accent px-6 py-2.5 text-sm font-medium text-white transition-colors hover:bg-accent/90 disabled:opacity-50"
          >
            {health?.status === "healthy" ? "Check again" : "Verify connection"}
          </button>
          <div className="mt-4">
            <button
              onClick={() => setStep(2)}
              className="text-xs text-text-secondary hover:text-text-primary"
            >
              ← Back
            </button>
          </div>
        </div>
      )}

      {/* ── Step 4: Done ────────────────────────────── */}
      {step === 4 && (
        <div className="text-center">
          <h2 className="text-xl font-semibold">You're all set</h2>
          <p className="mt-3 text-sm text-text-secondary">
            Open Claude Code, Cursor, OpenCode, or any AI coding tool and start
            working. SessionGraph will start saving your context
            automatically.
          </p>
          <p className="mt-4 text-sm text-text-secondary">
            Supported providers: <strong>Anthropic</strong>,{" "}
            <strong>OpenAI</strong>, <strong>OpenRouter</strong>, and any
            OpenAI-compatible endpoint.
          </p>
          {venvSetupDone && (
            <p className="mt-2 text-xs text-text-secondary">
              Setting up compression environment in background...
            </p>
          )}
          <button
            onClick={handleComplete}
            className="mt-8 rounded-lg bg-accent px-8 py-3 text-sm font-medium text-white transition-colors hover:bg-accent/90"
          >
            Open Dashboard
          </button>
        </div>
      )}
    </main>
  );
}
