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
      {/* Settings button — only shown on dashboard, inline in the layout */}
      {view === "dashboard" && (
        <div className="absolute top-0 right-0 z-10 px-6 py-[26px]">
          <button
            onClick={() => setView("settings")}
            className="rounded-md px-2.5 py-1 text-xs text-text-secondary/60 transition-colors hover:bg-surface hover:text-text-secondary"
            aria-label="Settings"
          >
            ⚙ Settings
          </button>
        </div>
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
