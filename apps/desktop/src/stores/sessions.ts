// Sessions list store (Zustand).
// Stub: real data wiring lands in Week 2.
import { create } from "zustand";

export type SessionSummary = {
  id: string;
  projectHash: string;
  projectName: string | null;
  provider: string;
  tool: string | null;
  startedAt: string;
  endedAt: string | null;
  tokensSaved: number;
  costSavedUsd: number;
  hasGraph: boolean;
};

type SessionsState = {
  sessions: SessionSummary[];
  setSessions: (sessions: SessionSummary[]) => void;
};

export const useSessionsStore = create<SessionsState>((set) => ({
  sessions: [],
  setSessions: (sessions) => set({ sessions }),
}));
