-- Migration 001: initial schema. See spec section 4.

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,                    -- UUID v4
  project_hash TEXT NOT NULL,             -- SHA256 truncated to 16 chars
  project_name TEXT,                      -- inferred or null
  provider TEXT NOT NULL,                 -- 'anthropic' | 'openai' | 'other'
  tool TEXT,                              -- 'claude-code' | 'cursor' | 'windsurf' | null
  started_at TEXT NOT NULL,               -- ISO8601
  ended_at TEXT,                          -- ISO8601, null if active
  status TEXT NOT NULL DEFAULT 'active',  -- 'active' | 'ended' | 'extracted'
  message_count INTEGER DEFAULT 0,
  tokens_in_raw INTEGER DEFAULT 0,        -- tokens before compression
  tokens_in_sent INTEGER DEFAULT 0,       -- tokens actually sent (after compression)
  tokens_out INTEGER DEFAULT 0,
  cost_usd_raw REAL DEFAULT 0,            -- cost if no compression
  cost_usd_actual REAL DEFAULT 0,         -- actual cost paid
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE requests (
  id TEXT PRIMARY KEY,                    -- UUID v4
  session_id TEXT NOT NULL REFERENCES sessions(id),
  sequence INTEGER NOT NULL,              -- request number within session
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  tokens_in_raw INTEGER NOT NULL,
  tokens_in_sent INTEGER NOT NULL,
  tokens_out INTEGER NOT NULL,
  compression_ratio REAL,                 -- tokens_in_sent / tokens_in_raw
  graph_injected INTEGER DEFAULT 0,       -- boolean: was a graph injected
  graph_tokens INTEGER DEFAULT 0,         -- tokens used by injected graph
  latency_ms INTEGER,
  cost_usd_raw REAL NOT NULL,
  cost_usd_actual REAL NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE session_graphs (
  id TEXT PRIMARY KEY,                    -- UUID v4
  session_id TEXT NOT NULL REFERENCES sessions(id),
  project_hash TEXT NOT NULL,
  graph_json TEXT NOT NULL,               -- the full SessionGraph JSON
  token_count INTEGER NOT NULL,           -- tokens in graph_json
  extraction_model TEXT NOT NULL,         -- model used for extraction
  extraction_cost_usd REAL NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  -- Only one graph per project_hash (latest wins)
  UNIQUE(project_hash) ON CONFLICT REPLACE
);

CREATE TABLE token_usage_daily (
  date TEXT NOT NULL,                     -- YYYY-MM-DD
  provider TEXT NOT NULL,
  tokens_in_raw INTEGER DEFAULT 0,
  tokens_in_sent INTEGER DEFAULT 0,
  tokens_out INTEGER DEFAULT 0,
  cost_usd_raw REAL DEFAULT 0,
  cost_usd_actual REAL DEFAULT 0,
  savings_usd REAL DEFAULT 0,
  PRIMARY KEY (date, provider)
);

CREATE TABLE settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Default settings
INSERT INTO settings (key, value) VALUES
  ('proxy_port', '4200'),
  ('session_timeout_minutes', '30'),
  ('compression_enabled', 'true'),
  ('graph_injection_enabled', 'true'),
  ('graph_max_tokens', '500'),
  ('tier', 'free'),
  ('sessions_saved_this_month', '0'),
  ('onboarding_complete', 'false');

-- Indices
CREATE INDEX idx_sessions_project_hash ON sessions(project_hash);
CREATE INDEX idx_sessions_started_at ON sessions(started_at);
CREATE INDEX idx_requests_session_id ON requests(session_id);
CREATE INDEX idx_requests_created_at ON requests(created_at);
CREATE INDEX idx_session_graphs_project_hash ON session_graphs(project_hash);
CREATE INDEX idx_token_usage_daily_date ON token_usage_daily(date);
