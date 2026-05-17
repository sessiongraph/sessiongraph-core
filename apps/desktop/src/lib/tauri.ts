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
  active_sessions: CurrentSession[];
};

export type CurrentSession = {
  id: string;
  active: boolean;
  tokens_in_raw: number;
  tokens_in_sent: number;
  compression_ratio: number;
  provider: string;
  project_name: string | null;
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

export type VenvStatus = {
  ready: boolean;
  python_path: string | null;
};

export type DailyTokenUsage = {
  date: string;
  tokens_raw: number;
  tokens_sent: number;
};

export type SystemProxyStatus = {
  enabled: boolean;
  pac_file_path: string;
};

// --- Commands -------------------------------------------------------------

export const tauri = {
  // stats
  getDashboardStats: () => invoke<DashboardStats>("get_dashboard_stats"),
  getCurrentSession: () => invoke<CurrentSession | null>("get_current_session"),
  getTokenUsageChart: (days: number) =>
    invoke<DailyTokenUsage[]>("get_token_usage_chart", { days }),

  // sessions
  listSessions: (page: number, perPage: number) =>
    invoke<SessionPage>("list_sessions", { page, perPage }),
  getSession: (id: string) => invoke<SessionSummary | null>("get_session", { id }),
  getSessionGraph: (projectHash: string) =>
    invoke<SessionGraph | null>("get_session_graph", { projectHash }),
  deleteSessionGraph: (projectHash: string) =>
    invoke<void>("delete_session_graph", { projectHash }),

  // settings
  getSettings: () => invoke<Settings>("get_settings"),
  updateSetting: (key: string, value: string) =>
    invoke<void>("update_setting", { key, value }),
  deleteAllData: () => invoke<void>("delete_all_data"),
  getAppVersion: () => invoke<string>("get_app_version"),

  // proxy control
  getProxyStatus: () => invoke<ProxyStatus>("get_proxy_status"),
  restartProxy: () => invoke<void>("restart_proxy"),

  // onboarding
  getSetupScript: () => invoke<string>("get_setup_script"),
  checkProxyHealth: () => invoke<HealthStatus>("check_proxy_health"),

  // venv / compression
  checkVenvStatus: () => invoke<VenvStatus>("check_venv_status"),
  setupVenv: () => invoke<string>("setup_venv"),

  // system proxy
  getSystemProxyStatus: () => invoke<SystemProxyStatus>("get_system_proxy_status"),
  setSystemProxy: (enabled: boolean) => invoke<void>("set_system_proxy", { enabled }),
};
