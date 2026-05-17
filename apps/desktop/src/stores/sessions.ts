// Sessions list store (Zustand).

import { create } from "zustand";
import { tauri, type SessionSummary, type SessionPage, type SessionGraph } from "../lib/tauri";
import { useNotificationsStore } from "./notifications";

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
      useNotificationsStore.getState().addNotification("Failed to load sessions");
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
      useNotificationsStore.getState().addNotification("Failed to load session graph");
    }
  },

  deleteGraph: async (projectHash: string) => {
    try {
      await tauri.deleteSessionGraph(projectHash);
      set((s) => ({
        sessions: s.sessions.map((item) =>
          item.project_hash === projectHash ? { ...item, has_graph: false } : item,
        ),
        selectedGraph: null,
        selectedProject: null,
      }));
      useNotificationsStore.getState().addNotification("Graph deleted", "success");
    } catch {
      useNotificationsStore.getState().addNotification("Failed to delete graph");
    }
  },
}));
