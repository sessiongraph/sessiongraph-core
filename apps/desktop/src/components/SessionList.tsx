// SessionList — list of saved sessions. See spec section 6.4.

import { useCallback, useEffect } from "react";
import { useSessionsStore } from "../stores/sessions";
import type { SessionSummary } from "../lib/tauri";

/** Format a USD amount. */
function fmtUsd(n: number): string {
  return `$${n.toFixed(2)}`;
}

/** Format tokens with K/M suffix. */
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
  return String(n);
}

/** Format an ISO8601 date string to a short locale date. */
function fmtDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return iso;
  }
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
  }, [fetchSessions]);

  const totalPages = Math.max(1, Math.ceil(total / perPage));

  const goToPage = useCallback(
    (p: number) => {
      if (p >= 1 && p <= totalPages) void fetchSessions(p);
    },
    [fetchSessions, totalPages],
  );

  return (
    <section className="mt-8">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-text-secondary">
          Recent Sessions
          {total > 0 && (
            <span className="ml-2 font-normal normal-case text-text-secondary/60">
              ({total})
            </span>
          )}
        </h2>
      </div>

      {loading && sessions.length === 0 && (
        <p className="mt-4 text-sm text-text-secondary">Loading sessions…</p>
      )}

      {!loading && sessions.length === 0 && (
        <p className="mt-4 text-sm text-text-secondary">
          No sessions recorded yet. Start coding with the proxy active to see them here.
        </p>
      )}

      {sessions.length > 0 && (
        <div className="mt-3 space-y-2">
          {sessions.map((s) => (
            <SessionRow
              key={s.id}
              session={s}
              isSelected={s.project_hash === selectedProject}
              onViewGraph={() => viewGraph(s.project_hash)}
            />
          ))}
        </div>
      )}

      {/* ── Pagination ──────────────────────────────── */}
      {totalPages > 1 && (
        <div className="mt-4 flex items-center justify-center gap-3">
          <button
            onClick={() => goToPage(page - 1)}
            disabled={page <= 1}
            className="rounded px-3 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-text-primary disabled:opacity-30 disabled:cursor-not-allowed"
          >
            ← Prev
          </button>
          <span className="text-xs text-text-secondary">
            {page} / {totalPages}
          </span>
          <button
            onClick={() => goToPage(page + 1)}
            disabled={page >= totalPages}
            className="rounded px-3 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-text-primary disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Next →
          </button>
        </div>
      )}
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
  const tokensSaved = session.tokens_in_raw - session.tokens_in_sent;
  const costSaved = session.cost_usd_raw - session.cost_usd_actual;

  return (
    <div
      className={`flex items-center gap-4 rounded-lg border px-4 py-3 transition-colors ${
        isSelected
          ? "border-accent bg-surface"
          : "border-border bg-surface hover:border-text-secondary/30"
      }`}
    >
      {/* Provider icon */}
      <span className="shrink-0 text-lg">
        {session.provider === "anthropic" ? "🟠" : "🟢"}
      </span>

      {/* Info */}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium text-text-primary">
          {session.project_name ?? session.project_hash.slice(0, 8)}
        </p>
        <p className="text-xs text-text-secondary">
          {session.provider}
          {session.tool && ` · ${session.tool}`}
          {" · "}
          {fmtDate(session.started_at)}
        </p>
      </div>

      {/* Stats */}
      <div className="hidden shrink-0 gap-4 text-right text-xs sm:flex">
        <div>
          <p className="text-text-secondary">Tokens saved</p>
          <p className="text-success">{fmtTokens(tokensSaved)}</p>
        </div>
        <div>
          <p className="text-text-secondary">Cost saved</p>
          <p className="text-success">{fmtUsd(costSaved)}</p>
        </div>
      </div>

      {/* Graph indicator */}
      <div className="shrink-0">
        {session.has_graph ? (
          <button
            onClick={onViewGraph}
            className="rounded bg-accent/10 px-2 py-1 text-xs font-medium text-accent transition-colors hover:bg-accent/20"
            title="View session graph"
          >
            ▦ Graph
          </button>
        ) : (
          <span className="text-xs text-text-secondary/40">—</span>
        )}
      </div>
    </div>
  );
}
