// Dashboard live stats store (Zustand).
// Polls the Rust backend periodically for live token/cost stats.

import { create } from "zustand";
import { tauri, type DashboardStats } from "../lib/tauri";

type DashboardState = {
  stats: DashboardStats | null;
  connected: boolean;
  /** Fetch latest stats from the proxy backend. */
  fetchStats: () => Promise<void>;
};

export const useDashboardStore = create<DashboardState>((set) => ({
  stats: null,
  connected: false,

  fetchStats: async () => {
    try {
      const stats = await tauri.getDashboardStats();
      set({ stats, connected: true });
    } catch {
      set({ connected: false });
    }
  },
}));

// Re-export the canonical types
export type { DashboardStats, CurrentSession } from "../lib/tauri";
