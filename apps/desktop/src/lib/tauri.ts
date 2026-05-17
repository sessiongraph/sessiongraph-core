// Typed Tauri invoke wrappers. See spec section 7.
import { invoke } from "@tauri-apps/api/core";

// --- Types ----------------------------------------------------------------

export type DashboardStats = {
  today: {
    tokens_saved: number;
    cost_saved_usd: number;
    requests: number;
    sessions: number;
  };
  total: {
    tokens_saved: number;
    cost_saved_usd: number;
    sessions: number;
  };
  current_session: CurrentSession | null;
};

export type CurrentSession = {
  id: string;
  active: boolean;
  tokens_in_raw: number;
  tokens_in_sent: number;
  compression_ratio: number;
};

export type SessionSummary = {
  id: string;
  project_hash: string;
  project_name: string | null;
  provider: string;
  tool: string | null;
  started_at: string;
  ended_at: string | null;
  tokens_in_raw: number;
  tokens_in_sent: number;
  cost_usd_raw: number;
  cost_usd_actual: number;
  has_graph: boolean;
};

export type SessionPage = {
  items: SessionSummary[];
  page: number;
  per_page: number;
  total: number;
};

export type SessionDetail = SessionSummary & {
  message_count: number;
  tokens_out: number;
};

export type SessionGraph = {
  sg_version: string;
  session_id: string;
  project_hash: string;
  created_at: string;
  last_updated: string;
  token_count: number;
  project: Record<string, unknown>;
  state: Record<string, unknown>;
  decisions: Array<Record<string, unknown>>;
  conventions: Record<string, unknown>;
  files: Record<string, unknown>;
  errors: Array<Record<string, unknown>>;
};

export type Settings = Record<string, string>;

export type ProxyStatus = {
  running: boolean;
  port: number;
  uptime_seconds: number;
};

export type HealthStatus = {
  status: "healthy" | "unhealthy";
  proxy_version: string;
  uptime_seconds: number;
};

// --- Commands -------------------------------------------------------------

export const tauri = {
  // stats
  getDashboardStats: () => invoke<DashboardStats>("get_dashboard_stats"),
  getCurrentSession: () => invoke<CurrentSession | null>("get_current_session"),

  // sessions
  listSessions: (page: number, perPage: number) =>
    invoke<SessionPage>("list_sessions", { page, perPage }),
  getSession: (id: string) => invoke<SessionDetail>("get_session", { id }),
  getSessionGraph: (projectHash: string) =>
    invoke<SessionGraph | null>("get_session_graph", { projectHash }),
  deleteSessionGraph: (projectHash: string) =>
    invoke<void>("delete_session_graph", { projectHash }),

  // settings
  getSettings: () => invoke<Settings>("get_settings"),
  updateSetting: (key: string, value: string) =>
    invoke<void>("update_setting", { key, value }),

  // proxy control
  getProxyStatus: () => invoke<ProxyStatus>("get_proxy_status"),
  restartProxy: () => invoke<void>("restart_proxy"),

  // onboarding
  getSetupScript: () => invoke<string>("get_setup_script"),
  checkProxyHealth: () => invoke<HealthStatus>("check_proxy_health"),
};
