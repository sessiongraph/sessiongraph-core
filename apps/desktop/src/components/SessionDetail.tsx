import { useState, type ReactNode } from "react";
import { useSessionsStore } from "../stores/sessions";
import GraphViz from "./GraphViz";

export default function SessionDetail() {
  const { selectedGraph, selectedProject, deleteGraph, fetchSessions } =
    useSessionsStore();
  const [showGraph, setShowGraph] = useState(false);

  if (!selectedGraph) return null;

  const handleDelete = async () => {
    if (selectedProject) {
      await deleteGraph(selectedProject);
      await fetchSessions();
    }
  };

  const decisionCount = arr(selectedGraph.decisions).length;
  const activeFiles = arr((selectedGraph.files as Record<string, unknown>).active);
  const readFiles = arr((selectedGraph.files as Record<string, unknown>).read);
  const createdFiles = arr((selectedGraph.files as Record<string, unknown>).created);
  const totalFiles = activeFiles.length + readFiles.length + createdFiles.length;

  return (
    <section className="space-y-3">
      <div className="flex items-center justify-between">
        <h2 className="text-[10px] font-semibold uppercase tracking-widest text-text-secondary/60">
          Session Graph
        </h2>
        <div className="flex items-center gap-1.5">
          <button
            onClick={() => setShowGraph(!showGraph)}
            className="rounded px-2.5 py-1 text-xs text-accent transition-colors hover:bg-accent/10"
          >
            {showGraph ? "Card view" : "Graph view"}
          </button>
          <button
            onClick={handleDelete}
            className="rounded px-2.5 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-red-400"
          >
            Delete
          </button>
        </div>
      </div>

      {showGraph ? (
        <GraphViz graph={selectedGraph} />
      ) : (
        <div className="space-y-3">
          {/* Where you left off */}
          <GraphCard>
            <SectionHeader emoji="📍" title="Where you left off" />
            <div className="mt-3 space-y-2">
              <Field label="Current task" value={str(selectedGraph.state.current_task)} />
              <Field label="Progress" value={str(selectedGraph.state.progress)} />
              {renderList("Next steps", selectedGraph.state.next_steps)}
              {renderList("Blockers", selectedGraph.state.blockers, "text-amber-400")}
            </div>
          </GraphCard>

          {/* Decisions */}
          {decisionCount > 0 && (
            <GraphCard>
              <SectionHeader
                emoji="✅"
                title="Decisions made"
                count={decisionCount}
              />
              <div className="mt-3 divide-y divide-border">
                {arr(selectedGraph.decisions).map((d, i) => {
                  const decision = d as Record<string, unknown>;
                  return (
                    <div key={i} className="py-2.5 first:pt-0 last:pb-0">
                      <p className="text-sm font-medium text-text-primary">
                        {str(decision.topic)}
                      </p>
                      <p className="text-sm text-text-secondary mt-0.5">
                        {str(decision.decision)}
                      </p>
                      {decision.rationale != null && (
                        <p className="mt-1 text-xs italic text-text-secondary/60">
                          {str(decision.rationale)}
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
            </GraphCard>
          )}

          {/* Files */}
          {totalFiles > 0 && (
            <GraphCard>
              <SectionHeader emoji="📁" title="Active files" count={totalFiles} />
              <div className="mt-3 space-y-2">
                <FileChips label="Active" files={(selectedGraph.files as Record<string, unknown>).active} />
                <FileChips label="Read" files={(selectedGraph.files as Record<string, unknown>).read} />
                <FileChips label="Created" files={(selectedGraph.files as Record<string, unknown>).created} />
              </div>
            </GraphCard>
          )}

          {/* Conventions */}
          <GraphCard>
            <SectionHeader emoji="🔧" title="Conventions" />
            <div className="mt-3 space-y-2">
              <Field label="Naming" value={str(selectedGraph.conventions.naming)} />
              <Field label="Structure" value={str(selectedGraph.conventions.structure)} />
              {renderList("Patterns", selectedGraph.conventions.patterns)}
            </div>
          </GraphCard>

          {/* Errors — only if present */}
          {arr(selectedGraph.errors).length > 0 && (
            <GraphCard>
              <SectionHeader
                emoji="⚠️"
                title="Errors"
                count={arr(selectedGraph.errors).length}
              />
              <div className="mt-3 divide-y divide-border">
                {arr(selectedGraph.errors).map((e, i) => {
                  const err = e as Record<string, unknown>;
                  return (
                    <div key={i} className="py-2.5 first:pt-0 last:pb-0">
                      {err.file != null && (
                        <p className="font-mono text-xs text-accent mb-0.5">
                          {str(err.file)}
                        </p>
                      )}
                      <p className="text-sm text-text-primary">
                        {str(err.description)}
                      </p>
                      {err.resolution != null && (
                        <p className="mt-0.5 text-xs text-success">
                          Resolved: {str(err.resolution)}
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
            </GraphCard>
          )}

          {/* Footer */}
          <p className="text-center text-[10px] text-text-secondary/40 tabular-nums">
            {selectedGraph.token_count} tokens · extracted{" "}
            {fmtDate(selectedGraph.created_at)} · v{selectedGraph.sg_version}
          </p>
        </div>
      )}
    </section>
  );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function str(v: unknown): string {
  if (v == null) return "—";
  if (typeof v === "string") return v || "—";
  if (Array.isArray(v)) return v.map(String).join(", ");
  return String(v);
}

function arr(v: unknown): unknown[] {
  return Array.isArray(v) ? v : [];
}

function fmtDate(iso: string): string {
  try {
    return new Date(iso).toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
    });
  } catch {
    return iso;
  }
}

function renderList(label: string, value: unknown, textClass?: string): ReactNode {
  if (!Array.isArray(value) || value.length === 0) return null;
  return (
    <div>
      <p className="text-xs text-text-secondary/60 mb-1">{label}</p>
      <ul
        className={`list-inside list-disc text-sm space-y-0.5 ${
          textClass ?? "text-text-primary"
        }`}
      >
        {(value as string[]).map((item, i) => (
          <li key={i}>{String(item)}</li>
        ))}
      </ul>
    </div>
  );
}

// ── Sub-components ─────────────────────────────────────────────────────────

function GraphCard({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-border bg-surface px-5 py-4">
      {children}
    </div>
  );
}

function SectionHeader({
  emoji,
  title,
  count,
}: {
  emoji: string;
  title: string;
  count?: number;
}) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-base leading-none">{emoji}</span>
      <h3 className="text-sm font-semibold text-text-primary">{title}</h3>
      {count !== undefined && count > 0 && (
        <span className="rounded-full bg-border px-1.5 py-0.5 text-[9px] text-text-secondary tabular-nums">
          {count}
        </span>
      )}
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  if (!value || value === "—") return null;
  return (
    <div>
      <span className="text-xs text-text-secondary/60">{label}: </span>
      <span className="text-sm text-text-primary">{value}</span>
    </div>
  );
}

function FileChips({ label, files }: { label: string; files: unknown }) {
  if (!Array.isArray(files) || files.length === 0) return null;
  return (
    <div>
      <p className="text-xs text-text-secondary/60 mb-1">{label}</p>
      <div className="flex flex-wrap gap-1">
        {(files as string[]).map((f, i) => (
          <code
            key={i}
            className="rounded bg-background px-1.5 py-0.5 font-mono text-xs text-accent"
          >
            {f}
          </code>
        ))}
      </div>
    </div>
  );
}
