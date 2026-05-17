// Dashboard — primary view. See spec section 6.3.

import { useEffect, useState } from "react";
import { useDashboardStore } from "../stores/dashboard";
import { tauri, type DailyTokenUsage, type ProxyStatus } from "../lib/tauri";
import SessionList from "./SessionList";
import SessionDetail from "./SessionDetail";

/** Format a USD amount with 2 decimal places. */
function fmtUsd(n: number): string {
  return `$${n.toFixed(2)}`;
}

/** Format a token count with K/M suffix. */
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

/** Short ordinal string (e.g. "42", "3 sessions"). */
function fmtCount(n: number, unit: string): string {
  return `${n} ${unit}${n !== 1 ? "s" : ""}`;
}

export default function Dashboard() {
  const { stats, connected, fetchStats } = useDashboardStore();
  const [chartData, setChartData] = useState<DailyTokenUsage[]>([]);
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);

  // Fetch proxy status once at mount for dynamic port display
  useEffect(() => {
    void tauri.getProxyStatus().then(setProxyStatus);
  }, []);

  // Poll the backend every 5 seconds
  useEffect(() => {
    void fetchStats();
    const id = setInterval(() => void fetchStats(), 5_000);
    return () => clearInterval(id);
  }, [fetchStats]);

  // Fetch 7-day chart data
  useEffect(() => {
    void tauri.getTokenUsageChart(7).then(setChartData).catch(() => {});
  }, [stats?.today?.requests]);

  const today = stats?.today;
  const total = stats?.total;
  const activeSessions = stats?.active_sessions ?? [];

  return (
    <main className="mx-auto max-w-5xl px-8 py-10">
      {/* ── Header ─────────────────────────────────────── */}
      <header className="flex items-center justify-between border-b border-border pb-4">
        <h1 className="text-xl font-semibold tracking-tight">SessionGraph</h1>
        <div className="flex items-center gap-2 text-sm">
          <span
            className={`inline-block h-2 w-2 rounded-full ${connected ? "bg-success" : "bg-text-secondary"}`}
          />
          <span className="text-text-secondary">
            {connected ? "Proxy Active" : "Connecting…"}
          </span>
        </div>
      </header>

      {/* ── Today stat cards ──────────────────────────── */}
      <section className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-3">
        <StatCard
          label="Saved Today"
          value={today ? fmtUsd(today.cost_saved_usd) : "—"}
          sub={today ? fmtTokens(today.tokens_saved) + " tokens" : undefined}
          accent="success"
        />
        <StatCard
          label="Compression"
          value={
            activeSessions.length > 0 && activeSessions.some(s => s.tokens_in_raw > 0)
              ? `${((1 - activeSessions.reduce((a, s) => a + (s.tokens_in_raw > 0 ? s.compression_ratio : 1), 0) / activeSessions.length) * 100).toFixed(0)}%`
              : "—"
          }
          sub={activeSessions.length > 0 ? "avg ratio" : undefined}
        />
        <StatCard
          label="Requests"
          value={today ? String(today.requests) : "—"}
          sub={today ? fmtCount(today.tokens_saved > 0 ? today.sessions : 0, "session") : undefined}
        />
      </section>

      {/* ── Monthly savings bar ──────────────────────── */}
      <section className="mt-8 rounded-lg border border-border bg-surface p-5">
        <div className="flex items-center justify-between text-sm">
          <span className="text-text-secondary">THIS MONTH</span>
          <span className="font-medium text-success">
            {total ? fmtUsd(total.cost_saved_usd) + " saved" : "—"}
          </span>
        </div>
        {total && total.tokens_saved > 0 ? (
          <div className="mt-3 h-2 w-full rounded-full bg-border">
            <div
              className="h-2 rounded-full bg-success transition-all duration-700"
              style={{
                width: `${Math.min(
                  (total.cost_saved_usd / (total.cost_saved_usd + 5)) * 100,
                  100,
                )}%`,
              }}
            />
          </div>
        ) : (
          <p className="mt-2 text-xs text-text-secondary">
            Start coding — savings will appear here as requests flow through the proxy.
          </p>
        )}
      </section>

      {/* ── Live sessions ───────────────────────────── */}
      {stats?.active_sessions && stats.active_sessions.length > 0 && (
        <section className="mt-6 space-y-3">
          <span className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
            Live {stats.active_sessions.length === 1 ? "Session" : "Sessions"}
          </span>
          {stats.active_sessions.map((s) => (
            <div key={s.id} className="rounded-lg border border-border bg-surface p-5">
              <div className="flex items-center gap-2 mb-2">
                <span className={`inline-block h-2 w-2 rounded-full ${s.provider === "anthropic" ? "bg-amber-400" : "bg-accent"}`} />
                <span className="text-xs font-medium text-text-primary">
                  {s.project_name ?? s.id.slice(0, 8)}
                </span>
                <span className="text-xs text-text-secondary">
                  · {s.provider}
                </span>
              </div>
              <div className="grid grid-cols-2 gap-4 text-sm sm:grid-cols-4">
                <div>
                  <p className="text-text-secondary">Session</p>
                  <p className="font-mono text-xs text-text-primary">
                    {s.id.slice(0, 8)}…
                  </p>
                </div>
                <div>
                  <p className="text-text-secondary">Tokens Sent</p>
                  <p className="text-text-primary">{fmtTokens(s.tokens_in_sent)}</p>
                </div>
                <div>
                  <p className="text-text-secondary">Raw Would Be</p>
                  <p className="text-text-primary">{fmtTokens(s.tokens_in_raw)}</p>
                </div>
                <div>
                  <p className="text-text-secondary">Saving</p>
                  <p className="text-success">
                    {s.tokens_in_raw > 0
                      ? `${((1 - s.compression_ratio) * 100).toFixed(0)}%`
                      : "—"}
                  </p>
                </div>
              </div>
            </div>
          ))}
        </section>
      )}

      {/* ── Empty state ──────────────────────────────── */}
      {(!stats?.active_sessions || stats.active_sessions.length === 0) && (
        <section className="mt-8 text-center">
          <p className="text-text-secondary">
            No active session detected. Point your AI coding tool at
            <code className="mx-1 rounded bg-surface px-1.5 py-0.5 text-sm text-accent">
              http://localhost:{proxyStatus?.port ?? 4200}
            </code>
            and start a conversation.
          </p>
          <p className="mt-1 text-xs text-text-secondary">
            Set <code className="rounded bg-surface px-1 py-0.5 text-xs">ANTHROPIC_BASE_URL</code>
            {" "}or{" "}
            <code className="rounded bg-surface px-1 py-0.5 text-xs">OPENAI_BASE_URL</code>
            {" "}to the proxy address.
          </p>
        </section>
      )}

      {/* ── 7-day token usage chart ────────────────── */}
      {chartData.length > 0 && (
        <section className="mt-8 rounded-lg border border-border bg-surface p-5">
          <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary mb-4">
            Token Usage — Last 7 Days
          </h3>
          <TokenChart data={chartData} />
        </section>
      )}

      <SessionList />
      <SessionDetail />
    </main>
  );
}

// ── Stat card ──────────────────────────────────────────────────

function StatCard({
  label,
  value,
  sub,
  accent,
}: {
  label: string;
  value: string;
  sub?: string;
  accent?: "success" | "accent";
}) {
  const valueColor =
    accent === "success"
      ? "text-success"
      : accent === "accent"
        ? "text-accent"
        : "text-text-primary";

  return (
    <div className="rounded-lg border border-border bg-surface p-5">
      <p className="text-xs font-medium uppercase tracking-wider text-text-secondary">
        {label}
      </p>
      <p className={`mt-2 text-2xl font-semibold ${valueColor}`}>{value}</p>
      {sub && <p className="mt-1 text-xs text-text-secondary">{sub}</p>}
    </div>
  );
}

// ── Token usage bar chart (inline SVG) ────────────────────────────

function TokenChart({ data }: { data: DailyTokenUsage[] }) {
  const maxTokens = Math.max(...data.map((d) => Math.max(d.tokens_raw, d.tokens_sent)), 1);
  const w = 500;
  const h = 140;
  const barW = Math.max(8, Math.floor((w - 40) / data.length / 2) - 2);

  return (
    <svg viewBox={`0 0 ${w} ${h}`} className="w-full h-auto">
      {data.map((d, i) => {
        const xRaw = 30 + i * (barW * 2 + 6);
        const xSent = xRaw + barW + 2;
        const hRaw = Math.max(2, (d.tokens_raw / maxTokens) * (h - 24));
        const hSent = Math.max(2, (d.tokens_sent / maxTokens) * (h - 24));

        return (
          <g key={d.date}>
            <rect
              x={xRaw}
              y={h - hRaw - 14}
              width={barW}
              height={hRaw}
              fill="#71717a"
              rx={1}
            >
              <title>Raw: {fmtTokens(d.tokens_raw)}</title>
            </rect>
            <rect
              x={xSent}
              y={h - hSent - 14}
              width={barW}
              height={hSent}
              fill="#22c55e"
              rx={1}
            >
              <title>Sent: {fmtTokens(d.tokens_sent)}</title>
            </rect>
            <text
              x={xRaw + barW}
              y={h - 2}
              textAnchor="middle"
              className="fill-text-secondary"
              fontSize="9"
            >
              {d.date.slice(5)}
            </text>
          </g>
        );
      })}
      {/* Legend */}
      <rect x={30} y={2} width={8} height={8} fill="#71717a" rx={1} />
      <text x={42} y={10} className="fill-text-secondary" fontSize="9">Raw</text>
      <rect x={70} y={2} width={8} height={8} fill="#22c55e" rx={1} />
      <text x={82} y={10} className="fill-text-secondary" fontSize="9">Compressed</text>
    </svg>
  );
}
