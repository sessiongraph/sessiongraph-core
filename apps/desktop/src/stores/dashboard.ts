// Dashboard live stats store (Zustand).
// Stub: real data wiring lands in Week 1 Task 10.
import { create } from "zustand";

export type DashboardStats = {
  today: {
    tokensSaved: number;
    costSavedUsd: number;
    requests: number;
    sessions: number;
  };
  total: {
    tokensSaved: number;
    costSavedUsd: number;
    sessions: number;
  };
};

type DashboardState = {
  stats: DashboardStats | null;
  setStats: (stats: DashboardStats) => void;
};

export const useDashboardStore = create<DashboardState>((set) => ({
  stats: null,
  setStats: (stats) => set({ stats }),
}));
