import React, { useMemo, useRef, useState } from "react";
import {
  Focus,
  Minus,
  Plus,
  Workflow,
  Bot,
  Maximize2,
  Info,
  Layers,
  Code2,
  Sparkles,
  Orbit,
  Terminal,
} from "lucide-react";
import { Button } from "./components/ui/button";
import { Card } from "./components/ui/card";
import { Dialog } from "./dialog";
import { Status, coordinatorStatus } from "./status";

const stride = 238;
const rowHeight = 98;
const nodeWidth = 205;

// Dependencies advance from left to right. Workers keep their own horizontal lanes.
function layout(tasks, workers) {
  const nodes = new Map(
    tasks.map((task, order) => [task.id, { task, order, column: 0, next: [], remaining: 0 }]),
  );
  const edges = [];
  function connect(from, to) {
    if (!nodes.has(from) || !nodes.has(to)) return;
    const a = nodes.get(from),
      b = nodes.get(to);
    a.next.push(b);
    b.remaining++;
    edges.push([a, b]);
  }
  for (const task of tasks) {
    for (const id of task.depends || []) connect(id, task.id);
    if (task.parent) connect(task.id, task.parent);
  }
  const ready = [...nodes.values()].filter((n) => !n.remaining);
  for (const node of ready) {
    for (const next of node.next) {
      next.column = Math.max(next.column, node.column + 1);
      if (--next.remaining === 0) ready.push(next);
    }
  }
  const visible = [...nodes.values()];
  const columns = [...new Set(visible.map((node) => node.column))].sort((a, b) => a - b);
  const positions = new Map(columns.map((depth, index) => [depth, index]));
  let y = 16,
    width = stride;

  for (const worker of workers) {
    const counts = new Map();
    for (const node of visible) {
      if (node.task.worker !== worker) continue;
      const row = counts.get(node.column) || 0;
      node.x = 16 + stride * positions.get(node.column);
      node.y = y + row * rowHeight;
      width = Math.max(width, node.x + nodeWidth + 16);
      counts.set(node.column, row + 1);
    }
    y += Math.max(0, ...counts.values()) * rowHeight + 14;
  }
  return { width, height: y, edges, nodes: visible };
}

function chain(edges, selected) {
  if (!selected) return null;
  const result = new Set([selected]);
  for (const forward of [true, false]) {
    const links = new Map();
    for (const [from, to] of edges) {
      const [a, b] = forward ? [from.task.id, to.task.id] : [to.task.id, from.task.id];
      if (!links.has(a)) links.set(a, []);
      links.get(a).push(b);
    }
    const pending = [selected],
      seen = new Set(pending);
    while (pending.length) {
      for (const id of links.get(pending.pop()) || []) {
        if (!seen.has(id)) {
          seen.add(id);
          result.add(id);
          pending.push(id);
        }
      }
    }
  }
  return result;
}

export function AssignmentMap({ run, tasks, offset, onDetails, renderDetails }) {
  const [zoom, setZoom] = useState(1),
    [selected, select] = useState(null);
  const [agent, setAgent] = useState(""),
    [expanded, setExpanded] = useState(false);
  const [coordinator, setCoordinator] = useState(false);
  const viewport = useRef(null),
    drag = useRef(null),
    expandButton = useRef(null);
  const workers = useMemo(
    () => [...new Set([...run.input.workers, ...tasks.map((t) => t.worker)])],
    [tasks, run.input.workers],
  );
  const graph = useMemo(() => layout(tasks, workers), [tasks, workers]);
  const highlighted = useMemo(() => chain(graph.edges, selected), [graph, selected]);
  const faded = (t) => (highlighted ? !highlighted.has(t.id) : !!agent && t.worker !== agent);
  const shade = (worker) => (245 + workers.indexOf(worker) * 137.5) % 360;
  function focusTask(id) {
    setAgent("");
    select(selected === id ? null : id);
  }
  function fit() {
    const el = viewport.current;
    setZoom(Math.min(1, el.clientWidth / graph.width, el.clientHeight / graph.height));
    el.scrollTo(0, 0);
  }
  function startDrag(event) {
    if (event.pointerType === "touch" || event.button !== 0 || event.target.closest("button")) return;
    const el = event.currentTarget;
    drag.current = [event.clientX + el.scrollLeft, event.clientY + el.scrollTop];
    el.setPointerCapture(event.pointerId);
  }
  const map = (
    <Card className="assignment-map">
      <header>
        <div>
          <Workflow aria-hidden="true" />
          <h2>Agent map</h2>
        </div>
        <div className="map-tools">
          <Button
            variant="outline"
            size="icon"
            aria-label="Zoom out"
            disabled={zoom <= 0.15}
            onClick={() => setZoom((z) => Math.max(0.15, z / 1.25))}
          >
            <Minus />
          </Button>
          <output aria-label="Map zoom">{Math.round(zoom * 100)}%</output>
          <Button
            variant="outline"
            size="icon"
            aria-label="Zoom in"
            disabled={zoom >= 4}
            onClick={() => setZoom((z) => Math.min(4, z * 1.25))}
          >
            <Plus />
          </Button>
          {!expanded && (
            <Button ref={expandButton} variant="outline" onClick={() => setExpanded(true)}>
              <Maximize2 />
              Full screen
            </Button>
          )}
          <Button variant="outline" title="Fit all assignments" onClick={fit}>
            <Focus />
            Fit
          </Button>
        </div>
      </header>
      <Button
        variant="ghost"
        className="map-coordinator"
        aria-label="View coordinator activity"
        onClick={() => setCoordinator(true)}
      >
        <Workflow />
        <strong>{run.input.coordinator}</strong>
        <span>{coordinatorStatus(run)}</span>
      </Button>
      <div className="map-filter" role="group" aria-label="Filter assignments">
        <strong>Filter</strong>
        <Button
          size="sm"
          variant="outline"
          aria-pressed={!agent && !selected}
          onClick={() => {
            setAgent("");
            select(null);
          }}
        >
          <Layers />
          All
        </Button>
        {workers.map((w, i) => {
          const Icon =
            { codex: Code2, claude: Sparkles, antigravity: Orbit, hermes: Terminal }[
              run.profiles?.[w]?.kind
            ] || [Bot, Code2, Terminal, Orbit][i % 4];
          return (
            <Button
              key={w}
              size="sm"
              variant="outline"
              className="agent-filter"
              style={{ "--agent-hue": shade(w) }}
              aria-label={`Filter ${w} assignments`}
              aria-pressed={agent === w}
              onClick={() => {
                setAgent(agent === w ? "" : w);
                select(null);
              }}
            >
              <Icon />
              {w}
            </Button>
          );
        })}
        <small>{selected ? "Dependency chain" : graph.nodes.length + " milestones"} · Drag to pan</small>
      </div>
      {!tasks.length && <p>Assignments appear when the coordinator creates the plan.</p>}
      <div
        ref={viewport}
        className="map-viewport"
        role="region"
        aria-label="Assignment map, scroll to pan"
        tabIndex={0}
        onClick={(e) => {
          if (!e.target.closest("button")) select(null);
        }}
        onPointerDown={startDrag}
        onLostPointerCapture={() => {
          drag.current = null;
        }}
        onPointerMove={(event) => {
          if (drag.current)
            event.currentTarget.scrollTo(drag.current[0] - event.clientX, drag.current[1] - event.clientY);
        }}
      >
        <div style={{ width: graph.width * zoom, height: graph.height * zoom, overflow: "hidden" }}>
          <div
            className="map-world"
            style={{ width: graph.width, height: graph.height, transform: `scale(${zoom})` }}
          >
            <svg aria-hidden="true" width={graph.width} height={graph.height}>
              {graph.edges.map(([a, b], i) => {
                const x = a.x + nodeWidth,
                  y = a.y + 37,
                  end = b.y + 37;
                const mid = x + (b.x - x) / 2;
                return (
                  <path
                    key={i}
                    opacity={faded(a.task) || faded(b.task) ? 0.27 : 1}
                    d={`M ${x} ${y} C ${mid} ${y}, ${mid} ${end}, ${b.x} ${end}`}
                  />
                );
              })}
            </svg>
            {graph.nodes.map(({ task, x, y, order }) => (
              <div
                key={task.id}
                className="map-node"
                data-faded={faded(task)}
                data-selected={selected === task.id}
                style={{ left: x, top: y, width: nodeWidth, "--agent-hue": shade(task.worker) }}
              >
                <Button
                  variant="ghost"
                  className="map-select"
                  title="Highlight dependency chain"
                  aria-pressed={selected === task.id}
                  aria-label={`Milestone ${offset + order + 1}: ${task.goal}`}
                  onClick={() => focusTask(task.id)}
                >
                  <small>
                    {task.worker} · {offset + order + 1}
                  </small>
                  <strong>{task.goal}</strong>
                  <Status state={task.state} />
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  className="map-info"
                  aria-label={`Details for ${task.id}`}
                  title="Assignment details"
                  onClick={() => onDetails(task.id)}
                >
                  <Info />
                </Button>
              </div>
            ))}
          </div>
        </div>
      </div>
      {renderDetails(() => setExpanded(false))}
      {coordinator && (
        <Dialog title="Coordinator activity" onClose={() => setCoordinator(false)}>
          <p>{coordinatorStatus(run)}</p>
          <ol className="turn-history">
            {(run.turns || [])
              .filter((t) => ["plan", "observe", "repair", "chat", "pricing"].includes(t.role))
              .map((t) => (
                <li key={t.id}>
                  {t.role} · {t.agent || run.input.coordinator}
                  <Status state={t.state} />
                </li>
              ))}
          </ol>
        </Dialog>
      )}
    </Card>
  );
  return expanded ? (
    <Dialog
      title="Full-screen agent map"
      className="map-screen"
      onClose={() => {
        setExpanded(false);
        requestAnimationFrame(() => expandButton.current?.focus());
      }}
    >
      {map}
    </Dialog>
  ) : (
    map
  );
}
