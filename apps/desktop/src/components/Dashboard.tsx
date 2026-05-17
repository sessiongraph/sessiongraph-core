import { useEffect, useState } from "react";
import { useDashboardStore } from "../stores/dashboard";
import { useSessionsStore } from "../stores/sessions";
import { tauri, type DailyTokenUsage, type ProxyStatus } from "../lib/tauri";
import SessionList from "./SessionList";
import SessionDetail from "./SessionDetail";

function fmtUsd(n: number): string {
  if (n >= 1000) return `$${(n / 1000).toFixed(1)}k`;
  if (n < 0.01) return `$${n.toFixed(4)}`;
  return `$${n.toFixed(2)}`;
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function fmtRelative(iso: string): string {
  try {
    const d = new Date(iso);
    const diffMs = Date.now() - d.getTime();
    const diffH = diffMs / 3_600_000;
    if (diffH < 1) return `${Math.floor(diffMs / 60_000)}m ago`;
    if (diffH < 24) return `${Math.floor(diffH)}h ago`;
    if (diffH < 48) return "yesterday";
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
  } catch {
    return iso;
  }
}

export default function Dashboard() {
  const { stats, connected, fetchStats } = useDashboardStore();
  const { sessions } = useSessionsStore();
  const [chartData, setChartData] = useState<DailyTokenUsage[]>([]);
  const [proxyStatus, setProxyStatus] = useState<ProxyStatus | null>(null);

  useEffect(() => {
    void tauri.getProxyStatus().then(setProxyStatus);
  }, []);

  useEffect(() => {
    void fetchStats();
    const id = setInterval(() => void fetchStats(), 5_000);
    return () => clearInterval(id);
  }, [fetchStats]);

  useEffect(() => {
    let cancelled = false;
    void tauri
      .getTokenUsageChart(7)
      .then((data) => { if (!cancelled) setChartData(data); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [new Date().toISOString().slice(0, 10)]);

  const today = stats?.today;
  const total = stats?.total;
  const activeSessions = stats?.active_sessions ?? [];
  const port = proxyStatus?.port ?? 4200;

  const hasActivity = (today?.requests ?? 0) > 0;
  const hasTotal = (total?.sessions ?? 0) > 0;

  // Use server-side graph count from stats (always populated) as primary source.
  // Fall back to counting from the loaded sessions list for the "last restored" detail.
  const graphsSaved = (total?.graphs_saved ?? 0) > 0
    ? (total!.graphs_saved as number)
    : sessions.filter((s) => s.has_graph).length;
  const projectsWithMemory = new Set(
    sessions.filter((s) => s.has_graph).map((s) => s.project_hash),
  ).size;
  const memoryTokensSaved = graphsSaved * 400 * 0.85;
  const memoryCostSaved = (memoryTokensSaved / 1_000_000) * 3.0;
  const lastRestoredSession = sessions.find((s) => s.has_graph);

  const compressionAvg =
    chartData.length > 0
      ? (() => {
          const valid = chartData.filter((d) => d.tokens_raw > 0);
          if (valid.length === 0) return null;
          const avg =
            valid.reduce(
              (acc, d) => acc + (1 - d.tokens_sent / d.tokens_raw),
              0,
            ) / valid.length;
          return Math.round(avg * 100);
        })()
      : null;

  return (
    <main className="mx-auto max-w-5xl px-6 py-6 flex flex-col gap-5">
      {/* ── Header ──────────────────────────────────────────────────────── */}
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-2.5">
          <div className="flex h-7 w-7 items-center justify-center rounded-md bg-accent/15">
            <span className="text-accent text-xs font-bold">S</span>
          </div>
          <span className="text-sm font-semibold tracking-tight text-text-primary">
            SessionGraph
          </span>
        </div>
        <div className="flex items-center gap-4">
          <div className="flex items-center gap-1.5">
            <span
              className={`h-1.5 w-1.5 rounded-full ${
                connected ? "bg-success animate-pulse" : "bg-text-secondary/30"
              }`}
            />
            <span className="text-xs text-text-secondary">
              {connected ? `Proxy on :${port}` : "Connecting…"}
            </span>
          </div>
        </div>
      </header>

      {/* ── Active session banner (single collapsed bar) ─────────────────── */}
      {activeSessions.length > 0 && (() => {
        const s = activeSessions[0]!;
        const saving = s.tokens_in_raw > 0
          ? `${((1 - s.compression_ratio) * 100).toFixed(0)}% compression`
          : "intercepting…";
        const color = s.provider === "anthropic"
          ? "border-amber-400/25 bg-amber-400/8 text-amber-400"
          : s.provider === "openrouter"
            ? "border-purple-400/25 bg-purple-400/8 text-purple-400"
            : "border-emerald-400/25 bg-emerald-400/8 text-emerald-400";
        return (
          <div className={`flex items-center justify-between rounded-lg border px-4 py-2.5 ${color}`}>
            <div className="flex items-center gap-2 min-w-0">
              <span className="h-1.5 w-1.5 rounded-full bg-current animate-pulse shrink-0" />
              <span className="text-sm font-medium truncate">
                {s.project_name ?? `session ${s.id.slice(0, 6)}`}
              </span>
              {activeSessions.length > 1 && (
                <span className="text-xs opacity-60">+{activeSessions.length - 1} more</span>
              )}
            </div>
            <span className="text-xs font-medium shrink-0">{saving}</span>
          </div>
        );
      })()}

      {/* ── Two-panel stat area ──────────────────────────────────────────── */}
      <div className="grid grid-cols-2 gap-4">
        {/* Left: Token Savings */}
        <div className="rounded-lg border border-border bg-surface p-5 flex flex-col gap-4">
          <div>
            <p className="text-[10px] font-semibold uppercase tracking-widest text-text-secondary/60 mb-3">
              Token Savings
            </p>
            {hasActivity ? (
              <div className="flex items-end gap-5 flex-wrap">
                <div>
                  <p className="text-2xl font-semibold tabular-nums text-success">
                    {fmtUsd(today!.cost_saved_usd)}
                  </p>
                  <p className="text-xs text-text-secondary mt-0.5">saved today</p>
                </div>
                {compressionAvg !== null && (
                  <div>
                    <p className="text-2xl font-semibold tabular-nums text-text-primary">
                      {compressionAvg}%
                    </p>
                    <p className="text-xs text-text-secondary mt-0.5">compression avg</p>
                  </div>
                )}
                <div>
                  <p className="text-2xl font-semibold tabular-nums text-text-primary">
                    {today!.requests}
                  </p>
                  <p className="text-xs text-text-secondary mt-0.5">
                    {today!.requests === 1 ? "request" : "requests"}
                  </p>
                </div>
              </div>
            ) : (
              <p className="text-sm text-text-secondary/50">
                No requests yet — point your AI tool at{" "}
                <code className="font-mono text-accent text-xs">
                  localhost:{port}
                </code>
              </p>
            )}
          </div>

          {/* Sparkline */}
          <div className="flex-1">
            {chartData.some((d) => d.tokens_raw > 0) ? (
              <SparklineChart data={chartData} />
            ) : (
              <div className="h-16 flex items-center">
                <p className="text-xs text-text-secondary/40">
                  Chart appears after first requests
                </p>
              </div>
            )}
          </div>

          {hasTotal && (
            <p className="text-xs text-text-secondary/50 tabular-nums">
              {fmtUsd(total!.cost_saved_usd)} saved across {total!.sessions}{" "}
              {total!.sessions === 1 ? "session" : "sessions"} total
            </p>
          )}
        </div>

        {/* Right: Session Memory */}
        <div className="rounded-lg border border-border bg-surface p-5 flex flex-col gap-3">
          <p className="text-[10px] font-semibold uppercase tracking-widest text-text-secondary/60">
            Session Memory
          </p>

          {graphsSaved > 0 ? (
            <>
              <div className="flex items-end gap-5 flex-wrap">
                <div>
                  <p className="text-2xl font-semibold tabular-nums text-text-primary">
                    {graphsSaved}
                  </p>
                  <p className="text-xs text-text-secondary mt-0.5">
                    {graphsSaved === 1 ? "session" : "sessions"} with memory
                  </p>
                </div>
                {projectsWithMemory > 0 && (
                  <div>
                    <p className="text-2xl font-semibold tabular-nums text-text-primary">
                      {projectsWithMemory}
                    </p>
                    <p className="text-xs text-text-secondary mt-0.5">
                      {projectsWithMemory === 1 ? "project" : "projects"} with memory
                    </p>
                  </div>
                )}
              </div>

              <div className="border-t border-border pt-3 space-y-1">
                <p className="text-xs text-text-secondary/70">
                  Context rebuild saved:
                </p>
                <p className="text-sm font-medium text-text-primary tabular-nums">
                  ~{fmtTokens(Math.round(memoryTokensSaved))} tokens this month
                </p>
                <p className="text-xs text-success tabular-nums">
                  ≈ {fmtUsd(memoryCostSaved)}
                </p>
              </div>

              {lastRestoredSession && (
                <div className="border-t border-border pt-3">
                  <p className="text-xs text-text-secondary/70 mb-1">Last restored:</p>
                  <p className="text-sm font-medium text-text-primary truncate">
                    {lastRestoredSession.project_name ??
                      lastRestoredSession.project_hash.slice(0, 8)}
                    <span className="text-text-secondary/60 font-normal">
                      {" "}· {fmtRelative(lastRestoredSession.started_at)}
                    </span>
                  </p>
                </div>
              )}
            </>
          ) : (
            <p className="text-sm text-text-secondary/50 leading-relaxed">
              No session graphs yet. The proxy extracts context automatically
              as you code.
            </p>
          )}
        </div>
      </div>

      {/* ── Session list + detail ─────────────────────────────────────────── */}
      <SessionList />
      <SessionDetail />
    </main>
  );
}

// ── Sparkline area chart — compression ratio over 7 days ──────────────────

function SparklineChart({ data }: { data: DailyTokenUsage[] }) {
  const height = 64;
  const width = 100;
  const pts = data.map((d, i) => {
    const ratio = d.tokens_raw > 0 ? 1 - d.tokens_sent / d.tokens_raw : 0;
    const x = (i / (data.length - 1)) * width;
    const y = height - ratio * height;
    return { x, y, ratio, label: d.date.slice(5) };
  });

  const lastPt = pts[pts.length - 1];
  const pathD =
    pts
      .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`)
      .join(" ") +
    (lastPt ? ` L ${lastPt.x} ${height} L 0 ${height} Z` : "");

  const linePath = pts
    .map((p, i) => `${i === 0 ? "M" : "L"} ${p.x} ${p.y}`)
    .join(" ");

  return (
    <div className="space-y-1">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="w-full"
        style={{ height }}
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id="sparkGrad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-success)" stopOpacity="0.25" />
            <stop offset="100%" stopColor="var(--color-success)" stopOpacity="0.02" />
          </linearGradient>
        </defs>
        <path d={pathD} fill="url(#sparkGrad)" />
        <path
          d={linePath}
          fill="none"
          stroke="var(--color-success)"
          strokeWidth="1.5"
          vectorEffect="non-scaling-stroke"
        />
        {pts.map((p, i) => (
          <circle
            key={i}
            cx={p.x}
            cy={p.y}
            r="1.5"
            fill="var(--color-success)"
            opacity={p.ratio > 0 ? 1 : 0}
            vectorEffect="non-scaling-stroke"
          >
            <title>{`${p.label}: ${(p.ratio * 100).toFixed(0)}% compression`}</title>
          </circle>
        ))}
      </svg>
      <div className="flex justify-between">
        {data.map((d) => (
          <span key={d.date} className="text-[9px] text-text-secondary/40 tabular-nums">
            {d.date.slice(5)}
          </span>
        ))}
      </div>
    </div>
  );
}
