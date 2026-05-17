# Architecture

This document tracks design decisions and implementation notes that don't
belong in the master spec or README.

## Proxy Architecture

### Traffic interception

SessionGraph uses three complementary interception strategies:

1. **Environment variable overrides** (`setx`):
   - `ANTHROPIC_BASE_URL` → `http://127.0.0.1:4200`
   - `OPENAI_BASE_URL` → `http://127.0.0.1:4200`
   - These route HTTP API calls directly through the Axum proxy.

2. **System proxy (PAC file)**:
   - `AutoConfigURL` set to `~/.sessiongraph/proxy.pac`.
   - PAC file routes specific API domains (`api.anthropic.com`,
     `api.openai.com`, `api.deepseek.com`, etc.) through the proxy.
   - Non-API traffic bypasses the proxy.

3. **MITM TLS interception**:
   - Tools using `HTTPS_PROXY` (e.g. OpenCode, Antigravity) create CONNECT
     tunnels that the proxy cannot inspect.
   - SessionGraph terminates the client TLS using a self-signed CA that is
     automatically installed into the Windows CurrentUser Root store.
   - Decrypted HTTP/1.1 requests are parsed byte-by-byte and routed through
     the same processing pipeline (compression, injection, cost tracking).
   - Responses are written back as raw HTTP/1.1 bytes.

### CONNECT tunnel handling

When a CONNECT request arrives:

1. `connect_handler` in `server.rs` immediately returns `200 OK` to the
   client and spawns a task that waits for `hyper::upgrade::on(req)`.
2. If MITM is enabled, the upgraded connection is wrapped in
   `TokioIo<Upgraded>` and passed to `mitm::handle_connect` which performs
   TLS accept and HTTP routing.
3. If MITM is disabled, a raw TCP tunnel forwards bytes bidirectionally.

**Critical:** `hyper::upgrade::on(req)` must NOT be awaited in the handler —
the upgrade future can only resolve after the `200 OK` response has been
sent. Awaiting it creates a deadlock.

### MITM module (`proxy/mitm.rs`)

- **CA generation**: Uses `rcgen` to generate a self-signed root CA on first
  run, persisted to `~/.sessiongraph/mitm-ca.{crt,key}`.
- **CA installation**: On each startup, the CA certificate is imported into
  the Windows CurrentUser Root store via `powershell Import-Certificate`.
  No admin privileges required. Failure is best-effort (falls back to passthrough).
- **Per-host certificates**: Generated on-the-fly and cached in memory
  (`HashMap<String, CertificateAndKey>`). Each cert is signed by the CA for
  the specific hostname.
- **TLS accept**: Uses `tokio-rustls` `TlsAcceptor` to accept client TLS
  connections. The `Upgraded` stream is bridged through `TokioIo` (from
  `hyper-util`) since it implements `hyper::rt::Read/Write`, not
  `tokio::io::AsyncRead/AsyncWrite`.

### Request pipeline

```
Client → Axum proxy → detect_provider() → intercept_request()
  → compression (if enabled) → graph injection (if enabled)
  → forward upstream → intercept_response()
  → write response back to client
```

For MITM connections, the pipeline is the same but the entry point is
different:
```
Client TLS → TlsAcceptor accept → manual HTTP/1.1 parse
  → routing to handler → write response back as raw bytes
```

## Env var management

- On startup: `setx` sets `ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`,
  `HTTPS_PROXY`, `HTTP_PROXY`.
- On shutdown: `reg delete HKCU:...Environment` removes the user env vars.
- Failures are logged via `tracing::warn!` (not silently discarded).

## System proxy management

- On startup: `AutoConfigURL` set to the PAC file path.
- On shutdown: `AutoConfigURL` deleted.
- Uses `reg add` / `reg delete` with `CREATE_NO_WINDOW` flag.

## Shutdown safety (`lib.rs`)

`ProxyShutdown` holds an `Arc<InterceptState>` and ensures proper cleanup:

1. All active sessions are ended (extraction tasks spawned).
2. Proxy shutdown signal is sent.
3. Environment variables are removed.
4. System proxy is disabled.

Uses `Handle::try_current()` to avoid panic if the tokio runtime is already
torn down during app shutdown.

## Frontend Architecture

### State management (Zustand)

| Store | Purpose |
|---|---|
| `dashboard.ts` | Live polled stats (every 5s) |
| `sessions.ts` | Session list + pagination + graph detail |
| `notifications.ts` | Toast notifications (auto-dismiss 5s) |

### Components

| Component | Purpose |
|---|---|
| `Dashboard.tsx` | Main view: stat cards, compression, chart, live sessions |
| `SessionList.tsx` | Paginated session history with graph indicators |
| `SessionDetail.tsx` | Session graph viewer (card or graph view toggle) |
| `GraphViz.tsx` | D3.js force-directed graph of session context |
| `Toast.tsx` | Floating error/success notifications |
| `Settings.tsx` | App settings and data management |
| `Onboarding.tsx` | 4-step setup wizard |

### D3.js Graph Visualization

- Maps the structured session graph (state, decisions, errors, conventions,
  files) into a force-directed node-link diagram.
- Nodes are draggable; categories are color-coded.
- Links show relationships (e.g. errors connected to their associated files).

## Current implementation status

The proxy, MITM interception, session tracking, graph extraction/injection,
database layer, compression, and full frontend (dashboard, session list with
pagination, D3 graph visualization, settings, onboarding) are implemented and
tested. See the README for build status.
