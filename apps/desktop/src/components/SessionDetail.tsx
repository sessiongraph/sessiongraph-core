// SessionDetail — session graph viewer. See spec section 6.5.
// Renders the structured graph as readable cards, not raw JSON.

import type { ReactNode } from "react";
import { useSessionsStore } from "../stores/sessions";

export default function SessionDetail() {
  const { selectedGraph, selectedProject, deleteGraph, fetchSessions } =
    useSessionsStore();

  if (!selectedGraph) {
    return (
      <section className="mt-8 text-center text-sm text-text-secondary">
        Select a session above to view its context graph.
      </section>
    );
  }

  const handleDelete = async () => {
    if (selectedProject) {
      await deleteGraph(selectedProject);
      await fetchSessions();
    }
  };

  return (
    <section className="mt-8 space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-text-secondary">
          Session Graph
        </h2>
        <button
          onClick={handleDelete}
          className="rounded px-2 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-red-400"
        >
          Delete graph
        </button>
      </div>

      {/* State card */}
      <GraphCard title="Work State">
        <Field label="Current task" value={str(selectedGraph.state.current_task)} />
        <Field label="Progress" value={str(selectedGraph.state.progress)} />
        {renderList("Next steps", selectedGraph.state.next_steps)}
        {renderList("Blockers", selectedGraph.state.blockers, "text-amber-400")}
      </GraphCard>

      {/* Decisions card */}
      {arr(selectedGraph.decisions).length > 0 && (
        <GraphCard title="Decisions">
          {arr(selectedGraph.decisions).map((d, i) => (
            <div
              key={i}
              className="mt-2 border-t border-border pt-2 first:mt-0 first:border-0 first:pt-0"
            >
              <p className="text-sm font-medium text-text-primary">
                {str((d as Record<string, unknown>).topic)}
              </p>
              <p className="text-sm text-text-secondary">
                {str((d as Record<string, unknown>).decision)}
              </p>
              {(d as Record<string, unknown>).rationale != null && (
                <p className="mt-0.5 text-xs italic text-text-secondary/70">
                  {str((d as Record<string, unknown>).rationale)}
                </p>
              )}
            </div>
          ))}
        </GraphCard>
      )}

      {/* Conventions card */}
      <GraphCard title="Conventions">
        <Field label="Naming" value={str(selectedGraph.conventions.naming)} />
        <Field label="Structure" value={str(selectedGraph.conventions.structure)} />
        {renderList("Patterns", selectedGraph.conventions.patterns)}
      </GraphCard>

      {/* Files card */}
      <GraphCard title="Files">
        <FileList label="Active" files={selectedGraph.files.active} />
        <FileList label="Read" files={selectedGraph.files.read} />
        <FileList label="Created" files={selectedGraph.files.created} />
      </GraphCard>

      {/* Errors card */}
      {arr(selectedGraph.errors).length > 0 && (
        <GraphCard title="Errors">
          {arr(selectedGraph.errors).map((e, i) => {
            const err = e as Record<string, unknown>;
            return (
              <div
                key={i}
                className="mt-2 border-t border-border pt-2 first:mt-0 first:border-0 first:pt-0"
              >
                {err.file != null && (
                  <p className="font-mono text-xs text-accent">{str(err.file)}</p>
                )}
                <p className="text-sm text-text-primary">{str(err.description)}</p>
                {err.resolution != null && (
                  <p className="mt-0.5 text-xs text-success">
                    Resolved: {str(err.resolution)}
                  </p>
                )}
              </div>
            );
          })}
        </GraphCard>
      )}

      {/* Meta */}
      <p className="text-center text-xs text-text-secondary/50">
        v{selectedGraph.sg_version} · {selectedGraph.token_count} tokens ·{" "}
        extracted {fmtDate(selectedGraph.created_at)}
      </p>
    </section>
  );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function str(v: unknown): string {
  if (v == null) return "—";
  if (typeof v === "string") return v;
  if (Array.isArray(v)) return v.map((x) => String(x)).join(", ");
  return String(v);
}

function arr(v: unknown): unknown[] {
  if (Array.isArray(v)) return v;
  return [];
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
    <div className="mt-2">
      <span className="text-xs text-text-secondary">{label}</span>
      <ul className={`mt-1 list-inside list-disc text-sm ${textClass ?? "text-text-primary"}`}>
        {(value as string[]).map((item, i) => (
          <li key={i}>{String(item)}</li>
        ))}
      </ul>
    </div>
  );
}

// ── Sub-components ─────────────────────────────────────────────────────────

function GraphCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="rounded-lg border border-border bg-surface p-4">
      <h3 className="text-xs font-semibold uppercase tracking-wider text-text-secondary">
        {title}
      </h3>
      <div className="mt-2">{children}</div>
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-1 first:mt-0">
      <span className="text-xs text-text-secondary">{label}:</span>{" "}
      <span className="text-sm text-text-primary">{value}</span>
    </div>
  );
}

function FileList({ label, files }: { label: string; files: unknown }) {
  if (!Array.isArray(files) || files.length === 0) return null;
  return (
    <div className="mt-2 first:mt-0">
      <span className="text-xs text-text-secondary">{label}</span>
      <div className="mt-1 flex flex-wrap gap-1">
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
