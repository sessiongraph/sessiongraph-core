import { useCallback, useEffect } from "react";
import { useSessionsStore } from "../stores/sessions";
import type { SessionSummary } from "../lib/tauri";

function fmtUsd(n: number): string {
  if (n < 0.001) return `$${n.toFixed(5)}`;
  if (n < 0.01) return `$${n.toFixed(4)}`;
  if (n < 0.10) return `$${n.toFixed(3)}`;
  return `$${n.toFixed(2)}`;
}

function fmtDate(iso: string): string {
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

function ProviderBadge({ provider }: { provider: string }) {
  const styles: Record<string, string> = {
    anthropic: "bg-amber-400/10 text-amber-400 border-amber-400/20",
    openai: "bg-emerald-400/10 text-emerald-400 border-emerald-400/20",
    openrouter: "bg-purple-400/10 text-purple-400 border-purple-400/20",
    "openai-compatible": "bg-blue-400/10 text-blue-400 border-blue-400/20",
  };
  const label: Record<string, string> = {
    anthropic: "Anthropic",
    openai: "OpenAI",
    openrouter: "OpenRouter",
    "openai-compatible": "Custom",
  };
  return (
    <span
      className={`shrink-0 rounded border px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide leading-none ${
        styles[provider] ?? "bg-border text-text-secondary border-border"
      }`}
    >
      {label[provider] ?? provider}
    </span>
  );
}

export default function SessionList() {
  const {
    sessions,
    page,
    perPage,
    loading,
    total,
    fetchSessions,
    viewGraph,
    selectedProject,
  } = useSessionsStore();

  useEffect(() => {
    void fetchSessions(1);
    const id = setInterval(() => void fetchSessions(page), 5_000);
    return () => clearInterval(id);
  }, [fetchSessions, page]);

  const totalPages = Math.max(1, Math.ceil(total / perPage));

  const goToPage = useCallback(
    (p: number) => {
      if (p >= 1 && p <= totalPages) void fetchSessions(p);
    },
    [fetchSessions, totalPages],
  );

  return (
    <section>
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-2">
          <h2 className="text-[10px] font-semibold uppercase tracking-widest text-text-secondary/60">
            Recent Sessions
          </h2>
          {total > 0 && (
            <span className="rounded-full bg-border px-1.5 py-0.5 text-[9px] text-text-secondary tabular-nums">
              {total}
            </span>
          )}
        </div>
        {totalPages > 1 && (
          <div className="flex items-center gap-1.5">
            <button
              onClick={() => goToPage(page - 1)}
              disabled={page <= 1}
              className="rounded px-2 py-0.5 text-[10px] text-text-secondary transition-colors hover:bg-surface disabled:opacity-30 disabled:cursor-not-allowed"
            >
              ←
            </button>
            <span className="text-[10px] text-text-secondary/60 tabular-nums">
              {page}/{totalPages}
            </span>
            <button
              onClick={() => goToPage(page + 1)}
              disabled={page >= totalPages}
              className="rounded px-2 py-0.5 text-[10px] text-text-secondary transition-colors hover:bg-surface disabled:opacity-30 disabled:cursor-not-allowed"
            >
              →
            </button>
          </div>
        )}
      </div>

      <div
        className="rounded-lg border border-border bg-surface overflow-hidden"
        style={{ maxHeight: "calc(5 * 48px + 2px)" }}
      >
        {loading && sessions.length === 0 ? (
          <div className="divide-y divide-border">
            {[...Array(3)].map((_, i) => (
              <div key={i} className="h-12 animate-pulse bg-surface" />
            ))}
          </div>
        ) : sessions.length === 0 ? (
          <div className="flex items-center px-4 h-12">
            <p className="text-sm text-text-secondary/50">
              No sessions yet — start coding with the proxy active.
            </p>
          </div>
        ) : (
          <div className="overflow-y-auto" style={{ maxHeight: "calc(5 * 48px)" }}>
            <div className="divide-y divide-border">
              {sessions.map((s) => (
                <SessionRow
                  key={s.id}
                  session={s}
                  isSelected={s.project_hash === selectedProject}
                  onViewGraph={() => viewGraph(s.project_hash)}
                />
              ))}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}

function SessionRow({
  session,
  isSelected,
  onViewGraph,
}: {
  session: SessionSummary;
  isSelected: boolean;
  onViewGraph: () => void;
}) {
  const costSaved = session.cost_usd_raw - session.cost_usd_actual;
  const compressionPct =
    session.tokens_in_raw > 0
      ? Math.round((1 - session.tokens_in_sent / session.tokens_in_raw) * 100)
      : 0;

  return (
    <div
      className={`flex items-center gap-3 px-4 transition-colors ${
        isSelected ? "bg-accent/8" : "hover:bg-surface/60"
      }`}
      style={{ height: 48 }}
    >
      <ProviderBadge provider={session.provider} />

      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-text-primary leading-none mb-0.5">
          {session.project_name ?? (
            <span className="font-mono text-text-secondary text-xs">
              {session.project_hash.slice(0, 8)}
            </span>
          )}
        </p>
        <p className="text-[11px] text-text-secondary/60 leading-none">
          {session.tool && (
            <>
              <span>{session.tool}</span>
              <span className="mx-1 opacity-50">·</span>
            </>
          )}
          {fmtDate(session.started_at)}
          {session.ended_at == null && (
            <span className="ml-1.5 text-success">● live</span>
          )}
        </p>
      </div>

      <div className="hidden sm:flex items-center gap-3 shrink-0">
        {compressionPct > 1 && (
          <span className="text-xs font-medium text-success tabular-nums">
            {compressionPct}%
          </span>
        )}
        {costSaved > 0.00001 && (
          <span className="text-xs font-medium text-success tabular-nums">
            {fmtUsd(costSaved)}
          </span>
        )}
      </div>

      <div className="shrink-0 w-12 flex justify-end">
        {session.has_graph ? (
          <button
            onClick={onViewGraph}
            className={`rounded px-2 py-1 text-[11px] font-medium transition-colors ${
              isSelected
                ? "bg-accent text-white"
                : "bg-accent/10 text-accent hover:bg-accent/20"
            }`}
          >
            Graph
          </button>
        ) : (
          <span className="text-[11px] text-text-secondary/25">—</span>
        )}
      </div>
    </div>
  );
}
