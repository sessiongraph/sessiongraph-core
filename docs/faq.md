# SessionGraph — FAQ

## General

**What is SessionGraph?**

SessionGraph is a local desktop app that gives AI coding tools memory between sessions. It runs a proxy on your machine, intercepts AI API calls, and at the end of each session extracts a structured graph of what you built — files changed, decisions made, errors resolved, conventions established. The next time you open a session on the same project, that context is automatically injected so your AI picks up exactly where you left off.

**Does SessionGraph send my code to the cloud?**

No. The proxy runs entirely on your machine. Your code never leaves your computer. The session graph extraction calls your AI provider (Anthropic or OpenAI) using your existing API key — the same call you'd make anyway. SessionGraph does not have a server that receives your data.

**Does it slow down my AI responses?**

No. The proxy streams responses byte-for-byte as they arrive. Token counting and session tracking happen in background tasks that do not block the response. Latency overhead is under 1ms.

**Does it work if I'm offline?**

The proxy needs to forward requests to your AI provider, so it requires an internet connection for API calls. The dashboard, session history, and graph viewer work offline.

---

## Setup & Installation

**Do I need to configure anything after installing?**

No. Open the app once. It automatically:
- Generates and installs a MITM CA certificate to your Windows certificate store
- Sets registry env vars (`ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`, `CODEX_OSS_BASE_URL`) so new terminals pick them up
- Writes the opencode provider config
- Installs the PAC file for system proxy routing

Open a new terminal after first launch and supported tools will route through SessionGraph automatically.

**My AI tool can't connect after opening SessionGraph. What do I do?**

The env vars set by SessionGraph (`ANTHROPIC_BASE_URL`, `OPENAI_BASE_URL`) only apply to terminals opened *after* SessionGraph started. If you opened your terminal before launching SessionGraph, open a new terminal window.

If SessionGraph is closed and your tool still can't connect, the proxy env vars may still be set in the registry pointing to a stopped proxy. Restart SessionGraph, or open Settings and click "Remove proxy config" to clean them up.

**I closed SessionGraph and now Claude Code can't connect.**

When SessionGraph stops, it removes the env vars from the registry. But your *current terminal session* still has the old values. Either open a new terminal (which won't have the vars) or temporarily unset them:

```powershell
# PowerShell
Remove-Item Env:ANTHROPIC_BASE_URL -ErrorAction SilentlyContinue
```

**Does the installer work on a fresh Windows machine with no other tools installed?**

Yes. The MSI installer bundles everything needed. After installation, launch SessionGraph once to complete first-run setup (CA cert generation and installation, env var registration). No admin rights required for normal operation — the CA cert installs to the current user store, not the machine store.

---

## Tool Compatibility

**Which AI coding tools does SessionGraph work with?**

| Tool | Status | Notes |
|---|---|---|
| Claude Code | ✅ Full support | API key mode |
| opencode | ✅ Full support | All providers via config |
| Codex (OpenAI CLI) | ✅ API key mode only | Use `codex --set-api-key sk-...` |
| Aider | ✅ Full support | Reads `OPENAI_BASE_URL` |
| Continue | ✅ Full support | Reads `OPENAI_BASE_URL` |
| Cursor | ❌ Not supported | Proprietary backend |
| Windsurf | ❌ Not supported | Proprietary backend |
| Antigravity | ❌ Not supported | gRPC-web, no proxy path |

**Why doesn't SessionGraph work with Cursor/Windsurf?**

Cursor routes through `api2.cursor.sh` and Windsurf through `inference.codeium.com` — both are proprietary backends that don't expose a standard OpenAI-compatible API endpoint. There is no env var or config file that redirects them to a local proxy. This is a fundamental limitation of how these tools are built, not something SessionGraph can work around.

**Why doesn't Codex work with my ChatGPT Plus/Pro subscription?**

Codex in ChatGPT-login mode (`auth_mode: chatgpt`) authenticates via OAuth and sends requests to `chatgpt.com/backend-api/codex` — a proprietary protocol different from the public OpenAI API. SessionGraph can only intercept standard OpenAI API calls. If you have an OpenAI API key, run `codex --set-api-key sk-...` once to switch Codex to API key mode, and it will route through SessionGraph.

**opencode shows "unable to verify the first certificate" — how do I fix it?**

This was caused by `SSL_CERT_FILE` being set to our MITM CA cert, which replaced the entire Node.js CA bundle. This is fixed in the current version. If you're on an older version, update SessionGraph and re-run setup.

If the issue persists, check that `SSL_CERT_FILE` is not set in your terminal environment (`echo $SSL_CERT_FILE`). If it is, unset it and open a fresh terminal.

---

## Session Memory

**How does session memory work?**

At the end of a coding session (when there's 30 minutes of inactivity, or when you close the app), SessionGraph sends the conversation history to your AI provider and asks it to extract a structured graph: what you were working on, decisions made with their rationale, files actively touched, errors encountered and resolved, and coding conventions established. This graph is stored locally.

The next time you start a session on the same project (identified by your system prompt), SessionGraph injects this graph into the AI's context — up to 500 tokens by default. The AI now knows where you left off without you having to explain it.

**How is the project identified?**

By a hash of the system prompt. Each AI coding tool sends a system prompt that typically includes the project directory, tool version, and sometimes file contents. SessionGraph hashes this to create a stable project identifier. Same project = same hash = same session history.

**How many tokens does the memory injection use?**

Default is 500 tokens (configurable in Settings). At this size, injection is essentially free — the savings from not re-explaining context vastly exceed the injection cost. The dashboard shows the net estimated savings.

**Can I see what's in my session graph?**

Yes. In the Recent Sessions list, click the "Graph" button on any session that has a graph. You'll see a human-readable breakdown: where you left off, decisions made, active files, and conventions learned. You can also toggle to a D3 force-directed graph visualization.

**Can I delete a session graph?**

Yes. Open the session detail and click "Delete graph". This removes the graph from the database. The next session will start fresh without injected context.

---

## Token Savings & Cost

**How are token savings calculated?**

The proxy intercepts the request before it goes upstream. It measures the raw token count (what would have been sent), applies optional Headroom compression to reduce message size, and then sends the compressed version. The difference is the saving.

Token counts come directly from the API response's `usage` fields — not estimates. The Anthropic API returns `input_tokens` and `output_tokens` in the SSE stream. OpenAI returns `prompt_tokens` and `completion_tokens`. SessionGraph reads these from the stream as it passes through.

**The cost saved shows $0.00 even though tokens were saved — why?**

For very small saves (under $0.00001), the display rounds to zero. Update to the current version which shows sub-cent amounts with adaptive precision (e.g., `$0.0016`).

**What does the "Session Memory" panel's context rebuild estimate mean?**

When SessionGraph injects a session graph, it saves you from having to explain that context manually in your first message. The estimate assumes an average graph size of ~400 tokens and calculates the net saving (85% of graph size, accounting for the injection overhead itself). This is an approximation — actual savings vary by project size and session complexity.

---

## Privacy & Security

**The MITM certificate sounds alarming. What does it actually do?**

The certificate is used for TLS termination — it lets SessionGraph read the HTTP traffic between your tool and the AI provider. Without it, the traffic would be encrypted end-to-end and SessionGraph couldn't see it. The cert is generated locally (never leaves your machine), installed only to your user certificate store (not the machine store), and only intercepts connections to `api.anthropic.com`. All other HTTPS traffic passes through as a transparent tunnel without inspection.

**Does SessionGraph store my conversations?**

SessionGraph stores the request/response messages needed to extract session graphs — essentially the same content your AI tool already stores in its own history. This data is in a local SQLite database at `~/.sessiongraph/sessions.db`. Nothing is sent to any server other than your AI provider (which you're already paying).

**Can I delete all my data?**

Yes. Go to Settings → "Delete all data". This wipes the SQLite database. You can also manually delete `~/.sessiongraph/`.

---

## Troubleshooting

**The proxy shows as connected but no sessions appear.**

Confirm your AI tool is actually sending requests through the proxy. In a new terminal, check:
```powershell
echo $env:ANTHROPIC_BASE_URL   # should be http://localhost:4200
echo $env:OPENAI_BASE_URL      # should be http://localhost:4200/v1
```
If these are empty, the env vars weren't picked up — close the terminal, make sure SessionGraph is running, and open a fresh terminal.

**Compression shows "subprocess timed out after 15s".**

The Headroom compression subprocess (Python) failed to respond in time. This is usually caused by Python not being installed or the venv not being set up. Go to Settings → Setup Compression to initialize the Python environment. If you don't need compression (the session memory feature works without it), you can disable compression in Settings.

**The app crashes on startup.**

Check that port 4200 isn't already in use by another application. You can change the proxy port in Settings. Also check `~/.sessiongraph/app.log` for error details.

**I reinstalled and my old session graphs are gone.**

Session graphs are stored in `~/.sessiongraph/sessions.db`. If you uninstalled and deleted that folder, the data is gone. For future-proofing, back up `~/.sessiongraph/` before reinstalling.
