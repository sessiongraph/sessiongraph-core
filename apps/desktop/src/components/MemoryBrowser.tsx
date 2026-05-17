import { useEffect, useState, type ReactNode } from "react";
import { tauri, type GraphEntry, type SessionGraph } from "../lib/tauri";
import { useSessionsStore } from "../stores/sessions";
import GraphViz from "./GraphViz";

// ── Types ──────────────────────────────────────────────────────────────────

type ProjectInfo  = { name: string | null; stack: string[]; entry_points: string[]; package_manager: string | null };
type WorkState    = { current_task: string | null; progress: string | null; next_steps: string[]; blockers: string[] };
type Decision     = { topic: string; decision: string; rationale: string };
type Conventions  = { naming: string | null; structure: string | null; patterns: string[] };
type FilesInfo    = { active: string[]; read: string[]; created: string[] };
type ErrorEntry   = { file: string | null; description: string; resolution: string | null };

// ── Helpers ────────────────────────────────────────────────────────────────

function str(v: unknown): string {
  if (v == null) return "—";
  if (typeof v === "string") return v || "—";
  if (Array.isArray(v)) return v.map(String).join(", ");
  return String(v);
}

function arr<T>(v: unknown): T[] {
  return Array.isArray(v) ? (v as T[]) : [];
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
  } catch { return iso; }
}

function fmtTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

// ── Main component ─────────────────────────────────────────────────────────

export default function MemoryBrowser() {
  const [entries, setEntries] = useState<GraphEntry[]>([]);
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [graph, setGraph] = useState<SessionGraph | null>(null);
  const [graphLoading, setGraphLoading] = useState(false);
  const [showViz, setShowViz] = useState(false);
  const { deleteGraph } = useSessionsStore();

  // Load index
  useEffect(() => {
    void tauri.listGraphs().then(setEntries).catch(() => {});
  }, []);

  const filtered = entries.filter((e) => {
    if (!search) return true;
    const q = search.toLowerCase();
    return (
      (e.project_name ?? "").toLowerCase().includes(q) ||
      e.project_hash.includes(q) ||
      (e.current_task ?? "").toLowerCase().includes(q) ||
      e.stack.some((s) => s.toLowerCase().includes(q))
    );
  });

  const handleSelect = async (hash: string) => {
    if (hash === selected) return;
    setSelected(hash);
    setGraph(null);
    setShowViz(false);
    setGraphLoading(true);
    try {
      const g = await tauri.getSessionGraph(hash);
      setGraph(g);
    } finally {
      setGraphLoading(false);
    }
  };

  const handleDelete = async () => {
    if (!selected) return;
    await deleteGraph(selected);
    setEntries((prev) => prev.filter((e) => e.project_hash !== selected));
    setSelected(null);
    setGraph(null);
  };

  return (
    <section className="flex gap-3" style={{ minHeight: 380 }}>
      {/* ── Left: project list ─────────────────────────────────────────── */}
      <div className="flex flex-col w-56 shrink-0 gap-2">
        {/* Search */}
        <input
          type="text"
          placeholder="Search projects…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="w-full rounded-lg border border-border bg-surface px-3 py-1.5 text-xs text-text-primary placeholder:text-text-secondary/40 focus:outline-none focus:border-accent"
        />

        {/* List */}
        <div className="flex-1 rounded-lg border border-border bg-surface overflow-y-auto">
          {filtered.length === 0 ? (
            <div className="flex items-center justify-center h-24 px-3">
              <p className="text-xs text-text-secondary/50 text-center">
                {entries.length === 0
                  ? "No session memory saved yet."
                  : "No results."}
              </p>
            </div>
          ) : (
            <div className="divide-y divide-border">
              {filtered.map((e) => (
                <ProjectRow
                  key={e.project_hash}
                  entry={e}
                  selected={selected === e.project_hash}
                  onSelect={() => void handleSelect(e.project_hash)}
                />
              ))}
            </div>
          )}
        </div>

        {/* Entry count */}
        {entries.length > 0 && (
          <p className="text-[9px] text-text-secondary/40 tabular-nums text-center">
            {filtered.length} of {entries.length} project{entries.length !== 1 ? "s" : ""}
          </p>
        )}
      </div>

      {/* ── Right: graph detail ────────────────────────────────────────── */}
      <div className="flex-1 min-w-0">
        {!selected ? (
          <div className="h-full flex items-center justify-center rounded-lg border border-border bg-surface">
            <p className="text-sm text-text-secondary/40">
              Select a project to view its memory
            </p>
          </div>
        ) : graphLoading ? (
          <div className="h-full flex items-center justify-center rounded-lg border border-border bg-surface">
            <p className="text-xs text-text-secondary/50">Loading…</p>
          </div>
        ) : graph ? (
          <GraphDetail
            graph={graph}
            showViz={showViz}
            onToggleViz={() => setShowViz((v) => !v)}
            onDelete={handleDelete}
          />
        ) : (
          <div className="h-full flex items-center justify-center rounded-lg border border-border bg-surface">
            <p className="text-xs text-text-secondary/50">Failed to load graph.</p>
          </div>
        )}
      </div>
    </section>
  );
}

// ── Project row ────────────────────────────────────────────────────────────

function ProjectRow({
  entry,
  selected,
  onSelect,
}: {
  entry: GraphEntry;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      onClick={onSelect}
      className={`w-full text-left px-3 py-2.5 transition-colors ${
        selected ? "bg-accent/10" : "hover:bg-background"
      }`}
    >
      <p className={`text-sm font-medium truncate ${selected ? "text-accent" : "text-text-primary"}`}>
        {entry.project_name ?? (
          <span className="font-mono text-xs text-text-secondary">
            {entry.project_hash.slice(0, 8)}
          </span>
        )}
      </p>
      <div className="flex items-center gap-1.5 mt-0.5 flex-wrap">
        {entry.stack.slice(0, 3).map((s) => (
          <span
            key={s}
            className="rounded bg-background border border-border px-1 py-px text-[9px] text-text-secondary/70 leading-none"
          >
            {s}
          </span>
        ))}
      </div>
      <p className="text-[10px] text-text-secondary/50 mt-0.5 tabular-nums">
        {fmtTokens(entry.token_count)} tokens · {fmtDate(entry.created_at)}
      </p>
    </button>
  );
}

// ── Graph detail pane ──────────────────────────────────────────────────────

function GraphDetail({
  graph,
  showViz,
  onToggleViz,
  onDelete,
}: {
  graph: SessionGraph;
  showViz: boolean;
  onToggleViz: () => void;
  onDelete: () => void;
}) {
  const project = graph.project as ProjectInfo;
  const state   = graph.state as WorkState;
  const files   = graph.files as FilesInfo;
  const convs   = graph.conventions as Conventions;
  const decisions = arr<Decision>(graph.decisions);
  const errors    = arr<ErrorEntry>(graph.errors);

  const totalFiles =
    (files.active?.length ?? 0) +
    (files.read?.length ?? 0) +
    (files.created?.length ?? 0);

  return (
    <div className="flex flex-col gap-3 h-full overflow-y-auto pr-0.5">
      {/* Header bar */}
      <div className="flex items-center justify-between shrink-0">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold text-text-primary truncate">
            {project.name ?? graph.project_hash.slice(0, 8)}
          </h3>
          <p className="text-[10px] text-text-secondary/50 tabular-nums">
            {graph.token_count} tokens · updated {fmtDate(graph.last_updated)}
          </p>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <button
            onClick={onToggleViz}
            className="rounded px-2.5 py-1 text-xs text-accent transition-colors hover:bg-accent/10"
          >
            {showViz ? "Card view" : "Graph view"}
          </button>
          <button
            onClick={() => {
              if (confirm("Delete this memory graph? This cannot be undone.")) {
                void onDelete();
              }
            }}
            className="rounded px-2.5 py-1 text-xs text-text-secondary transition-colors hover:bg-surface hover:text-red-400"
          >
            Delete
          </button>
        </div>
      </div>

      {showViz ? (
        <div className="flex-1 rounded-lg border border-border bg-surface overflow-hidden" style={{ minHeight: 300 }}>
          <GraphViz graph={graph} />
        </div>
      ) : (
        <div className="space-y-3">
          {/* Where you left off */}
          <GraphCard>
            <SectionHeader emoji="📍" title="Where you left off" />
            <div className="mt-3 space-y-2">
              <Field label="Current task" value={str(state.current_task)} />
              <Field label="Progress" value={str(state.progress)} />
              {renderList("Next steps", state.next_steps)}
              {renderList("Blockers", state.blockers, "text-amber-400")}
            </div>
          </GraphCard>

          {/* Stack */}
          {(project.stack?.length ?? 0) > 0 && (
            <GraphCard>
              <SectionHeader emoji="🧱" title="Project" />
              <div className="mt-3 space-y-2">
                {project.package_manager && (
                  <Field label="Package manager" value={project.package_manager} />
                )}
                <div>
                  <p className="text-xs text-text-secondary/60 mb-1">Stack</p>
                  <div className="flex flex-wrap gap-1">
                    {project.stack.map((s) => (
                      <span key={s} className="rounded border border-border bg-background px-1.5 py-0.5 text-xs text-text-secondary">
                        {s}
                      </span>
                    ))}
                  </div>
                </div>
                {(project.entry_points?.length ?? 0) > 0 && (
                  <div>
                    <p className="text-xs text-text-secondary/60 mb-1">Entry points</p>
                    <div className="flex flex-wrap gap-1">
                      {project.entry_points.map((f) => (
                        <code key={f} className="rounded bg-background px-1.5 py-0.5 font-mono text-xs text-accent">
                          {f}
                        </code>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </GraphCard>
          )}

          {/* Decisions */}
          {decisions.length > 0 && (
            <GraphCard>
              <SectionHeader emoji="✅" title="Decisions made" count={decisions.length} />
              <div className="mt-3 divide-y divide-border">
                {decisions.map((d, i) => (
                  <div key={i} className="py-2.5 first:pt-0 last:pb-0">
                    <p className="text-sm font-medium text-text-primary">{d.topic}</p>
                    <p className="text-sm text-text-secondary mt-0.5">{d.decision}</p>
                    {d.rationale && (
                      <p className="mt-1 text-xs italic text-text-secondary/60">{d.rationale}</p>
                    )}
                  </div>
                ))}
              </div>
            </GraphCard>
          )}

          {/* Files */}
          {totalFiles > 0 && (
            <GraphCard>
              <SectionHeader emoji="📁" title="Active files" count={totalFiles} />
              <div className="mt-3 space-y-2">
                <FileChips label="Active" files={files.active} />
                <FileChips label="Read" files={files.read} />
                <FileChips label="Created" files={files.created} />
              </div>
            </GraphCard>
          )}

          {/* Conventions */}
          {(convs.naming || convs.structure || (convs.patterns?.length ?? 0) > 0) && (
            <GraphCard>
              <SectionHeader emoji="🔧" title="Conventions" />
              <div className="mt-3 space-y-2">
                <Field label="Naming" value={str(convs.naming)} />
                <Field label="Structure" value={str(convs.structure)} />
                {renderList("Patterns", convs.patterns)}
              </div>
            </GraphCard>
          )}

          {/* Errors */}
          {errors.length > 0 && (
            <GraphCard>
              <SectionHeader emoji="⚠️" title="Errors" count={errors.length} />
              <div className="mt-3 divide-y divide-border">
                {errors.map((e, i) => (
                  <div key={i} className="py-2.5 first:pt-0 last:pb-0">
                    {e.file && <p className="font-mono text-xs text-accent mb-0.5">{e.file}</p>}
                    <p className="text-sm text-text-primary">{e.description}</p>
                    {e.resolution && (
                      <p className="mt-0.5 text-xs text-success">Resolved: {e.resolution}</p>
                    )}
                  </div>
                ))}
              </div>
            </GraphCard>
          )}

          <p className="text-center text-[10px] text-text-secondary/40 tabular-nums pb-1">
            {graph.token_count} tokens · extracted {fmtDate(graph.created_at)} · v{graph.sg_version}
          </p>
        </div>
      )}
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

function SectionHeader({ emoji, title, count }: { emoji: string; title: string; count?: number }) {
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

function FileChips({ label, files }: { label: string; files: string[] | undefined }) {
  if (!files?.length) return null;
  return (
    <div>
      <p className="text-xs text-text-secondary/60 mb-1">{label}</p>
      <div className="flex flex-wrap gap-1">
        {files.map((f, i) => (
          <code key={i} className="rounded bg-background px-1.5 py-0.5 font-mono text-xs text-accent">
            {f}
          </code>
        ))}
      </div>
    </div>
  );
}

function renderList(label: string, value: string[] | undefined, textClass?: string): ReactNode {
  if (!value?.length) return null;
  return (
    <div>
      <p className="text-xs text-text-secondary/60 mb-1">{label}</p>
      <ul className={`list-inside list-disc text-sm space-y-0.5 ${textClass ?? "text-text-primary"}`}>
        {value.map((item, i) => <li key={i}>{item}</li>)}
      </ul>
    </div>
  );
}
