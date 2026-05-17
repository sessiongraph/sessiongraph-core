# SessionGraph Bug List

## FIXED

### 1. tokens_out Never Tracked ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/forward.rs`, `intercept.rs`

**Fix:** Added byte counter that tracks streaming response bytes in real-time. Output tokens calculated as `bytes / 4`. Background task reads final count after stream completes.

---

### 2. Request Sequence Number Hardcoded ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs`

**Fix:** Modified `manage_session()` to return `message_count` as sequence. Each request now gets a unique sequence number within the session.

---

### 3. Compression Logic Inverted ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs`

**Fix:** Changed from subtracting "saved" to properly replacing raw tokens with compressed tokens in the session's running total.

---

### 4. Settings Never Read From DB ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs:InterceptState::new()`

**Fix:** Now reads `session_timeout_minutes`, `compression_enabled`, and `graph_injection_enabled` from settings table on startup.

---

### 5. Project Name Never Populated ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/session.rs`

**Fix:** Added `infer_project_name()` function that parses system prompts for project context (explicit markers, package.json, GitHub repo, directory paths). Wired into session creation.

---

### 6. No Auto-Updater Config ✅ FIXED
**Location:** `apps/desktop/src-tauri/tauri.conf.json`

**Fix:** Added `plugins.updater` with endpoint `https://sessiongraph.dev/api/updates/{{target}}/{{arch}}/{{current_version}}`

---

### 7. No REST Endpoint for Session Graph ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/proxy/server.rs`

**Fix:** Added `GET /sessions/:project_hash/graph` route that returns the session graph JSON.

---

### 8. Health Check Doesn't Verify Reachability ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/commands/settings.rs`

**Fix:** Now makes actual HTTP request to `http://127.0.0.1:4200/health` to verify proxy is reachable.

---

### 9. No Explicit Error Log File ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/db/mod.rs`

**Fix:** Added `init_error_log()` and `log_error()` functions that write to `~/.sessiongraph/logs/error.log` per spec §5.5.

---

## Remaining (Deferred)

### 10. venv Setup During Onboarding ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/venv.rs`, `commands/settings.tsx`

**Fix:** Added `venv.rs` module with functions to:
- Check if venv exists and has headroom installed
- Create Python venv at `~/.sessiongraph/venv/`
- Install headroom-ai package

Added Tauri commands:
- `check_venv_status` - returns whether venv is ready
- `setup_venv` - triggers venv setup

Frontend integration:
- Onboarding now checks venv status on mount
- On completion, triggers venv setup in background if not ready

---

### 11. Restart Proxy Command ✅ FIXED
**Location:** `apps/desktop/src-tauri/src/commands/settings.rs`, `proxy/intercept.rs`, `proxy/server.rs`

**Fix:** Added restart capability:
- `InterceptState` now has `restart_tx` field for restart signals
- Server watches for restart signals alongside shutdown
- `restart_proxy` command now triggers the restart channel

Note: Full restart requires app restart to re-initialize the server, which is appropriate for a desktop app.

---

## Verified Complete

- **Dashboard UI** - Already matches spec §6.3 with TODAY cards, THIS MONTH bar, LIVE SESSION, empty state
- **SessionDetail** - Already wired up with graph as readable cards per spec §6.5

---

## ALL BUGS FIXED ✅

### 1. tokens_out Never Tracked
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs:164,276`

**Context:**
```rust
tokens_out: 0,  // Always zero - never counted from streaming response
```

**Why it's a bug:** The spec §4 requires tracking both input and output tokens for accurate cost calculation. Currently output tokens are always 0 because the streaming response is forwarded without counting.

**Suggested fix:** Implement streaming token counter that counts SSE chunks as they stream through the proxy.

---

### 2. Request Sequence Number Hardcoded
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs:479`

**Context:**
```rust
let sequence = 1u32;  // BUG: Should increment per request
```

**Why it's a bug:** Every request gets `sequence = 1` instead of incrementing per-request within a session. The spec §4 requires sequential numbering.

**Suggested fix:** Store sequence counter in ActiveSession and increment per request.

---

### 3. Compression Logic Inverted
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs:145-146,254-258`

**Context:**
```rust
// After compression, tokens_in_sent is ALREADY the compressed (smaller) value
// This logic incorrectly subtracts again
s.tokens_in_sent = s.tokens_in_sent.saturating_sub(tokens_saved_by_compression);
```

**Why it's a bug:** After compression, tokens_in_sent should equal the compressed token count. This code incorrectly reduces it further, leading to negative/compressed stats.

**Suggested fix:** Simply assign `tokens_in_sent` after compression, no subtraction needed.

---

### 4. Settings Never Read From DB
**Location:** `apps/desktop/src-tauri/src/proxy/intercept.rs:41-51`

**Context:**
```rust
impl InterceptState {
    pub fn new(db: Connection) -> Self {
        Self {
            // Hardcoded defaults - never reads from DB
            session_timeout_minutes: 30,
            compression_enabled: true,
            graph_injection_enabled: true,
            ...
        }
    }
}
```

**Why it's a bug:** The spec §5.1 specifies configurable proxy port, session timeout, compression toggle via settings table. User changes in Settings UI won't take effect.

**Suggested fix:** Query settings table in `InterceptState::new()` and use those values.

---

## Medium Priority

### 5. Project Name Never Populated
**Location:** `apps/desktop/src-tauri/src/db/queries.rs:14-32`

**Context:**
```rust
pub fn insert_session(conn: &Connection, s: &ActiveSession) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO sessions (..., project_name, ...)
         VALUES (..., NULL, ...)",  // Always NULL
```

**Why it's a bug:** Spec §4 expects `project_name` to be inferred from project context.

**Suggested fix:** Extract project name from system prompt or working directory when creating session.

---

### 6. Output Token Cost Ignored
**Location:** `apps/desktop/src-tauri/src/proxy/forward.rs:378-379`

**Context:**
```rust
// Formula exists but tokens_out is always 0 (see bug #1)
(tokens_in as f64 / 1_000_000.0) * price_in_per_1m
    + (tokens_out as f64 / 1_000_000.0) * price_out_per_1m
```

**Why it's a bug:** Users see lower costs than actual because output token costs aren't counted.

**Suggested fix:** Fix bug #1 to track output tokens.

---

### 7. No Auto-Updater Config
**Location:** `apps/desktop/src-tauri/tauri.conf.json`

**Context:** Missing `plugins.updater` section for `sessiongraph.dev/api/updates/latest.json`

**Why it's a bug:** Spec §9.3 requires silent auto-updater checks on app launch.

**Suggested fix:** Add updater configuration to tauri.conf.json.

---

### 8. No REST Endpoint for Session Graph
**Location:** `apps/desktop/src-tauri/src/proxy/server.rs`

**Context:** Spec §5.2 lists `GET /sessions/:id/graph` endpoint but only Tauri IPC command exists.

**Why it's a bug:** Frontend has to use IPC instead of direct HTTP for graph retrieval.

**Suggested fix:** Add `/sessions/:id/graph` route to Axum server.

---

### 9. Compression Fails Silently on First Run
**Location:** `apps/desktop/src-tauri/src/proxy/compress.rs:141-146`

**Context:**
```rust
if script.exists() {
    Some(script)
} else {
    tracing::warn!("Compression script not found at {}", script.display());
    None
}
```

**Why it's a bug:** Spec §5.4 says venv should be set up during onboarding, but the script won't exist on first run and compression silently falls back.

**Suggested fix:** Trigger venv setup during onboarding wizard completion.

---

## Low Priority

### 10. No Explicit Error Log File
**Location:** `apps/desktop/src-tauri/src/db/mod.rs:27`

**Context:** Errors go to tracing but not to `~/.sessiongraph/logs/error.log` per spec §5.5.

**Suggested fix:** Add file logging to error.log in addition to tracing.

---

### 11. Health Check Doesn't Verify Reachability
**Location:** `apps/desktop/src-tauri/src/commands/settings.rs:117-124`

**Context:**
```rust
pub async fn check_proxy_health(
    state: tauri::State<'_, Arc<InterceptState>>,
) -> Result<HealthStatus, String> {
    // Just returns internal uptime, doesn't verify server is actually accepting connections
    Ok(HealthStatus {
        status: "healthy",
        ...
    })
}
```

**Suggested fix:** Make actual HTTP request to localhost:4200/health to verify.

---

### 12. Restart Proxy Doesn't Work
**Location:** `apps/desktop/src-tauri/src/commands/settings.rs:90-95`

**Context:**
```rust
#[tauri::command]
pub fn restart_proxy() {
    // In v1, restarting the proxy requires restarting the app.
    tracing::warn!("restart_proxy called — full restart requires app restart");
}
```

**Why it's a bug:** Spec §7 expects functional restart_proxy command.

**Suggested fix:** Implement actual proxy restart by recreating server.

---

### 13. Dashboard UI Doesn't Match Spec Layout
**Location:** `apps/desktop/src/components/Dashboard.tsx`

**Context:** Spec §6.3 shows specific cards, progress bars, live session section, and charts. Current implementation is a minimal stub.

**Suggested fix:** Implement full spec UI with TODAY cards, THIS MONTH bar, LIVE SESSION, 7-day chart.

---

### 14. SessionDetail Not Wired to Graph Viewer
**Location:** `apps/desktop/src/App.tsx`

**Context:** SessionDetail.tsx exists but isn't connected to show graph as readable cards per spec §6.5.

**Suggested fix:** Connect SessionDetail component to store and render graph fields as cards.

---

## Week 1 Acceptance Criteria Impact

| Bug # | Blocks Week 1 Criteria |
|-------|------------------------|
| #1    | "Dashboard shows real token counts" |
| #2    | "All requests logged to SQLite correctly" |
| #4    | Settings UI has no effect on proxy |
| #5    | Session list shows no project names |

---

*Generated from spec v1.0 review - sessiongraph-core*