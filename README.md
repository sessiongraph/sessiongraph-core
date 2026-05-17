# SessionGraph

> Your AI remembers where you left off.

SessionGraph is a local desktop application and proxy server that gives AI
coding tools **persistent memory across sessions**. It automatically intercepts
AI API traffic from any tool on your machine, compresses session state into a
structured graph at session end, and injects that graph into the next session's
context — so your AI never forgets what you were working on.

## Features

- **Automatic traffic interception** — MITM TLS proxy intercepts all AI API
  calls (Anthropic, OpenAI, OpenRouter, DeepSeek, Gemini) from any tool:
  Claude Code, Cursor, Windsurf, OpenCode, Antigravity, Terax, and more.
- **One-click setup** — auto-installs system proxy (PAC file) and environment
  variables, no manual configuration required.
- **Session memory** — extracts a structured context graph (files, decisions,
  errors, conventions, work state) at session end and injects it into the
  next session.
- **Token & cost savings** — shows how many tokens and dollars you save by
  avoiding repeated context loss.
- **Live dashboard** — real-time stats, active sessions, 7-day token usage
  chart, and paginated session history.
- **D3.js graph visualization** — interactive force-directed graph of your
  session context (drag nodes, explore relationships between files, errors,
  decisions, and conventions).
- **Compression** — optional Headroom compression via Python subprocess to
  further reduce context size.

## Repository Layout

```
sessiongraph-core/
├── apps/
│   └── desktop/                # Tauri 2 + React 19 desktop application
│       ├── src/                # React frontend (Vite, Tailwind v4, Zustand, Recharts, D3)
│       │   ├── components/     # Dashboard, SessionList, SessionDetail, GraphViz, Toast, etc.
│       │   ├── stores/         # Zustand stores (dashboard, sessions, notifications)
│       │   └── lib/            # Typed Tauri IPC wrappers
│       └── src-tauri/          # Rust backend
│           ├── src/
│           │   ├── proxy/      # Axum HTTP proxy + MITM TLS interception
│           │   ├── db/         # SQLite (rusqlite, WAL mode, 5 tables)
│           │   ├── graph/      # Session graph extraction/injection
│           │   ├── commands/   # Tauri IPC command handlers
│           │   ├── lib.rs      # Entrypoint: Axum proxy + Tauri IPC
│           │   └── main.rs     # Thin wrapper
│           └── src-tauri/
├── packages/
│   └── sessiongraph-core/      # Shared npm package (future)
├── docs/                       # Architecture and onboarding docs
└── .github/workflows/          # CI and release pipelines
```

## Prerequisites

- Node.js >= 20
- pnpm >= 9
- Rust >= 1.77 (stable)
- Python >= 3.10 (for Headroom compression)

Platform-specific Tauri dependencies are documented in
[`docs/onboarding.md`](docs/onboarding.md).

## Getting Started

```bash
git clone https://github.com/sessiongraph/sessiongraph-core
cd sessiongraph-core
pnpm install
pnpm tauri:dev
```

This starts the Vite dev server for the React frontend and launches the Tauri
desktop window with the embedded Rust backend.

## How It Works

1. **Startup** — SessionGraph sets itself as the system proxy (PAC file at
   `~/.sessiongraph/proxy.pac`) and configures `ANTHROPIC_BASE_URL`,
   `OPENAI_BASE_URL`, `HTTPS_PROXY` environment variables via `setx`.
2. **Traffic interception** — HTTP requests are routed directly through the
   Axum proxy. HTTPS requests go through a MITM TLS tunnel where SessionGraph
   terminates the client TLS with a self-signed CA (auto-installed to the
   system trust store), inspects the plaintext, processes it through the
   compression/injection pipeline, then re-encrypts and forwards upstream.
3. **Session tracking** — Requests are grouped into sessions by
   `(project_hash, provider, api_key_hash)`. Sessions time out after 30 minutes
   of inactivity.
4. **Graph extraction** — At session end, the request/response history is sent
   to an LLM which extracts a structured session graph (files, decisions, errors,
   conventions, work state).
5. **Graph injection** — On the next session with the same project, the graph
   is injected into the system prompt (within a configurable token budget).
6. **Shutdown** — All active sessions are ended, graphs extracted, environment
   variables cleaned up, and system proxy disabled.

## Status

The proxy, MITM interception, session tracking, graph extraction/injection,
database layer, compression, and full frontend (dashboard, session list with
pagination, D3 graph visualization, settings, onboarding) are implemented and
tested. 68 Rust unit tests pass. `cargo fmt` and `cargo clippy -D warnings`
are clean.

## License

MIT
