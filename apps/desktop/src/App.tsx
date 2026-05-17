import { useEffect, useState } from "react";
import Dashboard from "./components/Dashboard";
import Onboarding from "./components/Onboarding";
import Settings from "./components/Settings";
import ToastContainer from "./components/Toast";
import { tauri } from "./lib/tauri";

type View = "onboarding" | "dashboard" | "settings";

export default function App() {
  const [view, setView] = useState<View | null>(null);

  useEffect(() => {
    void tauri
      .getSettings()
      .then((s) => {
        // Remember onboarding state in settings store
        const completed = s.onboarding_complete === "true";
        setView(completed ? "dashboard" : "onboarding");
      })
      .catch(() => setView("dashboard"));
  }, []);

  const handleOnboardingComplete = async () => {
    await tauri.updateSetting("onboarding_complete", "true");
    setView("dashboard");
  };

  if (view === null) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background">
        <span className="text-sm text-text-secondary">Loading…</span>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-background text-text-primary">
      {/* Top nav — only show on dashboard */}
      {view === "dashboard" && (
        <nav className="absolute top-0 right-0 px-8 py-4">
          <button
            onClick={() => setView("settings")}
            className="rounded px-3 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-text-primary"
          >
            Settings
          </button>
        </nav>
      )}

      {view === "onboarding" && (
        <Onboarding onComplete={handleOnboardingComplete} />
      )}
      {view === "dashboard" && <Dashboard />}
      {view === "settings" && (
        <Settings onClose={() => setView("dashboard")} />
      )}

      <ToastContainer />
    </div>
  );
}
