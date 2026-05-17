# SessionGraph

> Your AI remembers where you left off.

SessionGraph is a local desktop app and proxy that gives AI coding tools **persistent memory across sessions**. It automatically intercepts AI API calls, tracks what you built, and injects that context into your next session — so you spend less time re-explaining and more time building.

## What it does

- **Session memory** — at the end of each session, SessionGraph extracts a structured graph of your work (files touched, decisions made, errors resolved, conventions established). On your next session in the same project, that context is automatically injected into the system prompt.

- **Token & cost savings** — by compressing repeated context, the proxy reduces the tokens sent to the API. The dashboard shows exactly how many tokens and dollars you've saved.

- **Live dashboard** — two-panel view: Token Savings (compression ratio, cost saved, 7-day sparkline) and Session Memory (sessions restored, projects with memory, context rebuild savings, last restored preview).

- **Session graph viewer** — click any session with a graph to see a human-readable breakdown: where you left off, decisions made, active files, coding conventions learned, and errors resolved.

## Supported tools

| Tool | Status | Method |
|---|---|---|
| Claude Code | ✅ Works | `ANTHROPIC_BASE_URL` env var |
| opencode | ✅ Works | Config file (`~/.config/opencode/opencode.json`) |
| Codex (API key mode) | ✅ Works | `CODEX_OSS_BASE_URL` env var |
| Aider | ✅ Works | `OPENAI_BASE_URL` env var |
| Continue | ✅ Works | `OPENAI_BASE_URL` env var |
| Cursor | ❌ Cannot intercept | Routes through `api2.cursor.sh` (proprietary) |
| Windsurf | ❌ Cannot intercept | Routes through `inference.codeium.com` (proprietary) |
| Antigravity | ❌ Cannot intercept | gRPC-web HTTP/2 to proprietary backend |
| Codex (ChatGPT plan) | ❌ Cannot intercept | Uses `chatgpt.com/backend-api` (proprietary) |

## Fresh install — what happens automatically

When you install and open SessionGraph for the first time:

1. A MITM CA certificate is generated at `~/.sessiongraph/mitm-ca.crt` and automatically installed into the Windows certificate store (`certutil`). No manual steps.
2. Registry env vars are set via `setx`: `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`, `CODEX_OSS_BASE_URL`, `NODE_EXTRA_CA_CERTS`.
3. opencode's config file is written/merged at `~/.config/opencode/opencode.json`.
4. A PAC file is installed at `~/.sessiongraph/proxy.pac` for system proxy routing.

Open a new terminal after first launch and all tools that support env var configuration will route through SessionGraph automatically.

## Repository layout

```
sessiongraph-core/
├── apps/
│   └── desktop/                # Tauri 2 + React 19 desktop app
│       ├── src/                # React frontend (Vite, Tailwind v4, Zustand)
│       │   ├── components/     # Dashboard, SessionList, SessionDetail, GraphViz
│       │   ├── stores/         # Zustand stores (dashboard, sessions, notifications)
│       │   └── lib/            # Typed Tauri IPC wrappers
│       └── src-tauri/          # Rust backend
│           └── src/
│               ├── proxy/      # Axum HTTP proxy + MITM TLS + SSE token parsing
│               ├── db/         # SQLite (rusqlite, WAL mode)
│               ├── graph/      # Session graph extraction + injection
│               └── commands/   # Tauri IPC handlers + env var management
├── docs/                       # Architecture, onboarding, FAQ
└── .github/workflows/          # CI
```

## Development

```bash
git clone https://github.com/sessiongraph/sessiongraph-core
cd sessiongraph-core
pnpm install
pnpm tauri:dev
```

Prerequisites: Node.js ≥ 20, Rust ≥ 1.77, Python ≥ 3.10.

## Building the installer

```bash
cd apps/desktop
npm run tauri build
```

Output: `src-tauri/target/release/bundle/msi/SessionGraph_0.1.0_x64_en-US.msi`

## How the proxy works

1. **HTTP tools** (Claude Code, opencode) send plain HTTP `POST /v1/messages` or `POST /v1/chat/completions` directly to `localhost:4200` via env var overrides.
2. **HTTPS tools** (anything using HTTPS_PROXY) send `CONNECT api.anthropic.com:443` — SessionGraph terminates the TLS with a locally-signed cert (CA trusted via `certutil`), processes the request, re-encrypts, and forwards upstream.
3. The SSE response stream is teed: chunks go to the client immediately (no latency added), and a background task parses the `usage` fields from SSE events to get accurate token counts.
4. Sessions are grouped by `(project_hash, provider, api_key_hash)`. At session end (30-minute timeout or app shutdown), the conversation history is sent to an LLM to extract the session graph.
5. On the next session in the same project, the graph is injected into the system prompt within a configurable token budget (default 500 tokens).

## License

MIT
