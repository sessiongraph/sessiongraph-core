# SessionGraph

> Your AI remembers where you left off.

SessionGraph is a local desktop application and proxy server that gives AI
coding tools persistent memory across sessions. It intercepts AI API traffic
from any OpenAI-compatible tool on the developer's machine, compresses session
state into a structured graph at session end, and automatically injects that
state at the start of the next session.

## Repository Layout

```
sessiongraph-core/
├── apps/
│   └── desktop/                # Tauri 2 + React 19 desktop application
│       ├── src/                # React frontend
│       └── src-tauri/          # Rust backend (proxy, graph, db, commands)
├── packages/
│   └── sessiongraph-core/      # Shared npm package (future)
├── docs/                       # Architecture and onboarding docs
└── .github/workflows/          # CI and release pipelines
```

See the full specification for the design and roadmap.

## Prerequisites

- Node.js >= 20
- pnpm >= 9
- Rust >= 1.77 (stable)
- Python >= 3.10 (used later for Headroom compression)

Platform-specific Tauri dependencies are documented in
[`docs/onboarding.md`](docs/onboarding.md).

## Getting Started

```bash
pnpm install
pnpm tauri:dev
```

This starts the Vite dev server for the React frontend and launches the Tauri
desktop window with the embedded Rust backend.

## Status

Week 1 / Task 1 — project scaffold initialised. The proxy server, session
graph extractor, database layer, and dashboard are stubbed and will be filled
in over the weeks that follow.
