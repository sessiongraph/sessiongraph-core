import { useEffect, useRef } from "react";
import * as d3 from "d3";
import type { SessionGraph } from "../lib/tauri";

interface RawNode {
  id: string;
  label: string;
  type: string;
  group: string;
}

export default function GraphViz({ graph }: { graph: SessionGraph }) {
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    if (!svgRef.current) return;

    const rawNodes = buildGraph(graph);
    if (rawNodes.length === 0) return;

    const svg = d3.select(svgRef.current);
    svg.selectAll("*").remove();

    const w = svgRef.current.clientWidth || 600;
    const h = 400;

    const color = d3.scaleOrdinal(d3.schemeCategory10);

    const nodes: any = rawNodes.map((n) => ({ ...n }));
    const links: any = rawNodes
      .flatMap((n) =>
        n._links.map((t: string) => ({ source: n.id, target: t })),
      )
      .filter((l: any) => l.source !== l.target);

    const simulation = d3
      .forceSimulation(nodes)
      .force(
        "link",
        d3.forceLink(links).id((d: any) => d.id).distance(80),
      )
      .force("charge", d3.forceManyBody().strength(-300))
      .force("center", d3.forceCenter(w / 2, h / 2))
      .force("collision", d3.forceCollide().radius(30));

    const link = svg
      .append("g")
      .selectAll("line")
      .data(links)
      .join("line")
      .attr("stroke", "#2a2a2a")
      .attr("stroke-width", 1.5)
      .attr("stroke-opacity", 0.6);

    const node = svg
      .append("g")
      .selectAll("g")
      .data(nodes)
      .join("g")
      .call(
        d3.drag<any, any>()
          .on("start", (event, d) => {
            if (!event.active) simulation.alphaTarget(0.3).restart();
            d.fx = d.x;
            d.fy = d.y;
          })
          .on("drag", (event, d) => {
            d.fx = event.x;
            d.fy = event.y;
          })
          .on("end", (event, d) => {
            if (!event.active) simulation.alphaTarget(0);
            d.fx = null;
            d.fy = null;
          }),
      );

    node
      .append("circle")
      .attr("r", 6)
      .attr("fill", (d: any) => color(d.group))
      .attr("stroke", "#1a1a1a")
      .attr("stroke-width", 1.5);

    node
      .append("text")
      .text((d: any) => d.label)
      .attr("x", 10)
      .attr("y", 4)
      .attr("fill", "#a1a1aa")
      .style("font-size", "11px")
      .style("font-family", "Inter, sans-serif");

    node.append("title").text((d: any) => `${d.label} (${d.type})`);

    simulation.on("tick", () => {
      link
        .attr("x1", (d: any) => d.source.x)
        .attr("y1", (d: any) => d.source.y)
        .attr("x2", (d: any) => d.target.x)
        .attr("y2", (d: any) => d.target.y);

      node.attr("transform", (d: any) => `translate(${d.x},${d.y})`);
    });

    return () => {
      simulation.stop();
    };
  }, [graph]);

  return (
    <svg
      ref={svgRef}
      className="w-full rounded-lg border border-border bg-surface"
      style={{ height: 400, minHeight: 400 }}
    />
  );
}

function buildGraph(graph: SessionGraph): (RawNode & { _links: string[] })[] {
  const nodes: (RawNode & { _links: string[] })[] = [];
  const nodeMap = new Map<string, (RawNode & { _links: string[] })>();

  function addNode(
    id: string,
    label: string,
    type: string,
    group: string,
    linksTo: string[] = [],
  ) {
    if (!nodeMap.has(id)) {
      const n = { id, label, type, group, _links: [] as string[] };
      nodeMap.set(id, n);
      nodes.push(n);
    }
    const n = nodeMap.get(id)!;
    for (const t of linksTo) {
      if (t !== id && !n._links.includes(t)) {
        n._links.push(t);
      }
    }
  }

  const stateText = [
    graph.state?.current_task,
    graph.state?.progress,
  ]
    .filter(Boolean)
    .join(" \u00b7 ");
  const stateId = "state";
  addNode(stateId, stateText.slice(0, 40) || "State", "state", "State");
  addNode("state-root", "State", "state", "State", [stateId]);

  const steps = asStrArr(graph.state?.next_steps);
  steps.forEach((s, i) => {
    addNode(`step-${i}`, s.slice(0, 40), "step", "State", [stateId]);
  });

  const blockers = asStrArr(graph.state?.blockers);
  blockers.forEach((b, i) => {
    addNode(`blocker-${i}`, b.slice(0, 40), "blocker", "State", [stateId]);
  });

  const decisions = asArr(graph.decisions);
  decisions.forEach((d: any, i: number) => {
    const topic = str(d.topic) || `Decision ${i}`;
    const decisionId = `dec-${i}`;
    addNode(decisionId, topic.slice(0, 40), "decision", "Decisions");

    const detail = str(d.decision);
    if (detail) {
      addNode(`dec-detail-${i}`, detail.slice(0, 40), "decision-detail", "Decisions", [decisionId]);
    }
  });
  if (decisions.length > 0) {
    addNode(
      "decisions-root",
      `Decisions (${decisions.length})` as string,
      "category",
      "Decisions",
      decisions.map((_: any, i: number) => `dec-${i}`),
    );
  }

  const errors = asArr(graph.errors);
  errors.forEach((e: any, i: number) => {
    const file = str(e.file);
    const desc = str(e.description) || `Error ${i}`;
    const errId = `err-${i}`;
    const linked: string[] = [];
    if (file) {
      linked.push(`err-file-${i}`);
    }
    addNode(errId, desc.slice(0, 40), "error", "Errors");
    if (file) {
      addNode(`err-file-${i}`, file, "error-file", "Errors", [errId]);
    }
  });
  if (errors.length > 0) {
    addNode(
      "errors-root",
      `Errors (${errors.length})`,
      "category",
      "Errors",
      errors.map((_: any, i: number) => `err-${i}`),
    );
  }

  const naming = str(graph.conventions?.naming);
  const structure = str(graph.conventions?.structure);
  const patterns = asStrArr(graph.conventions?.patterns);

  if (naming || structure || patterns.length > 0) {
    addNode("conv-root", "Conventions", "category", "Conventions");
    if (naming) {
      addNode("conv-naming", `Naming: ${naming.slice(0, 40)}`, "convention", "Conventions", ["conv-root"]);
    }
    if (structure) {
      addNode("conv-structure", `Structure: ${structure.slice(0, 40)}`, "convention", "Conventions", ["conv-root"]);
    }
    patterns.forEach((p, i) => {
      addNode(`conv-pattern-${i}`, p.slice(0, 40), "convention", "Conventions", ["conv-root"]);
    });
  }

  const activeFiles = asStrArr(graph.files?.active);
  const readFiles = asStrArr(graph.files?.read);
  const createdFiles = asStrArr(graph.files?.created);

  if (activeFiles.length + readFiles.length + createdFiles.length > 0) {
    addNode("files-root", "Files", "category", "Files");
    activeFiles.forEach((f) => {
      addNode(`file-active-${f}`, f, "file", "Files", ["files-root"]);
    });
    readFiles.forEach((f) => {
      addNode(`file-read-${f}`, f, "file", "Files", ["files-root"]);
    });
    createdFiles.forEach((f) => {
      addNode(`file-created-${f}`, f, "file", "Files", ["files-root"]);
    });
  }

  errors.forEach((e: any, i: number) => {
    const file = str(e.file);
    if (!file) return;
    const errId = `err-${i}`;
    for (const prefix of ["file-active-", "file-read-", "file-created-"]) {
      const fileLink = `${prefix}${file}`;
      if (nodeMap.has(fileLink)) {
        const n = nodeMap.get(errId);
        if (n && !n._links.includes(fileLink)) {
          n._links.push(fileLink);
        }
      }
    }
  });

  return nodes;
}

function str(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  return String(v);
}

function asArr(v: unknown): unknown[] {
  if (Array.isArray(v)) return v;
  return [];
}

function asStrArr(v: unknown): string[] {
  if (Array.isArray(v)) return v.map((x) => String(x));
  return [];
}
