// Sessions list store (Zustand).

import { create } from "zustand";
import { tauri, type SessionSummary, type SessionPage, type SessionGraph } from "../lib/tauri";

type SessionsState = {
  sessions: SessionSummary[];
  page: number;
  perPage: number;
  total: number;
  /** Selected session's graph detail */
  selectedGraph: SessionGraph | null;
  selectedProject: string | null;
  loading: boolean;

  fetchSessions: (page?: number) => Promise<void>;
  viewGraph: (projectHash: string) => Promise<void>;
  deleteGraph: (projectHash: string) => Promise<void>;
};

export const useSessionsStore = create<SessionsState>((set, get) => ({
  sessions: [],
  page: 1,
  perPage: 20,
  total: 0,
  selectedGraph: null,
  selectedProject: null,
  loading: false,

  fetchSessions: async (page?: number) => {
    const p = page ?? get().page;
    set({ loading: true });
    try {
      const result: SessionPage = await tauri.listSessions(p, get().perPage);
      set({ sessions: result.items, page: result.page, total: result.total });
    } catch {
      // silently ignore — the UI shows empty state
    } finally {
      set({ loading: false });
    }
  },

  viewGraph: async (projectHash: string) => {
    try {
      const graph = await tauri.getSessionGraph(projectHash);
      set({ selectedGraph: graph, selectedProject: projectHash });
    } catch {
      set({ selectedGraph: null, selectedProject: null });
    }
  },

  deleteGraph: async (projectHash: string) => {
    await tauri.deleteSessionGraph(projectHash);
    set((s) => ({
      sessions: s.sessions.map((item) =>
        item.project_hash === projectHash ? { ...item, has_graph: false } : item,
      ),
      selectedGraph: null,
      selectedProject: null,
    }));
  },
}));
