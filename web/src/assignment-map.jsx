import React, { useMemo, useRef, useState } from "react";
import { Focus, Minus, Plus, Workflow, Bot } from "lucide-react";
import { Button } from "./components/ui/button";
import { Card } from "./components/ui/card";
import { Dialog } from "./dialog";
import { Status, coordinatorStatus } from "./status";
import { Markdown } from "./markdown";

const stride = 238;
const rowHeight = 98;
const nodeWidth = 205;

// Dependencies advance from left to right. Workers keep their own horizontal lanes.
function layout(tasks, workers) {
  const nodes = new Map(tasks.map((task, order) => [task.id, { task, order, column: 0, next: [], remaining: 0 }]));
  const edges = [];
  function connect(from, to) {
    if (!nodes.has(from) || !nodes.has(to)) return;
    const a = nodes.get(from), b = nodes.get(to);
    a.next.push(b);
    b.remaining++;
    edges.push([a, b]);
  }
  for (const task of tasks) {
    for (const id of task.depends || []) connect(id, task.id);
    if (task.parent) connect(task.id, task.parent);
  }
  const ready = [...nodes.values()].filter(n => !n.remaining);
  for (const node of ready) {
    for (const next of node.next) {
      next.column = Math.max(next.column, node.column + 1);
      if (--next.remaining === 0) ready.push(next);
    }
  }
  let y = 16, width = stride;
  const lanes = [];
  for (const worker of workers) {
    lanes.push({ worker, y });
    y += 46;
    const counts = new Map();
    for (const node of nodes.values()) {
      if (node.task.worker !== worker) continue;
      const row = counts.get(node.column) || 0;
      node.x = 16 + stride * node.column;
      node.y = y + row * rowHeight;
      width = Math.max(width, node.x + nodeWidth + 16);
      counts.set(node.column, row + 1);
    }
    y += Math.max(0, ...counts.values()) * rowHeight + 14;
  }
  return { width, height: y, lanes, edges, nodes: [...nodes.values()] };
}

export function AssignmentMap({ run }) {
  const [zoom, setZoom] = useState(1);
  const [selection, select] = useState(null);
  const viewport = useRef(null);
  const drag = useRef(null);
  const tasks = run.tasks || [];
  const graph = useMemo(() => layout(tasks, [...new Set([...run.input.workers, ...tasks.map(t => t.worker)])]), [tasks, run.input.workers]);
  const selectedTask = selection?.task && tasks.find(t => t.id === selection.task);
  const profile = run.profiles?.[selection?.worker];
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
  return <Card className="assignment-map">
    <header><div><Workflow aria-hidden="true" /><h2>Agent map</h2></div>
      <div className="map-tools">
        <Button variant="outline" size="icon" aria-label="Zoom out" disabled={zoom <= .15} onClick={() => setZoom(z => Math.max(.15, z / 1.25))}><Minus /></Button>
        <output aria-label="Map zoom">{Math.round(zoom * 100)}%</output>
        <Button variant="outline" size="icon" aria-label="Zoom in" disabled={zoom >= 2} onClick={() => setZoom(z => Math.min(2, z * 1.25))}><Plus /></Button>
        <Button variant="outline" title="Fit all assignments" onClick={fit}><Focus />Fit</Button>
      </div>
    </header>
    <Button variant="ghost" className="map-coordinator" aria-label="View coordinator activity" onClick={() => select({worker:run.input.coordinator, coordinator:true})}>
      <Workflow /><strong>{run.input.coordinator}</strong><span>{coordinatorStatus(run)}</span>
    </Button>
    {!tasks.length && <p>Assignments appear when the coordinator creates the plan.</p>}
    <div ref={viewport} className="map-viewport" role="region" aria-label="Assignment map, scroll to pan" tabIndex={0}
      onPointerDown={startDrag} onLostPointerCapture={() => { drag.current = null; }}
      onPointerMove={event => {
        if (drag.current) event.currentTarget.scrollTo(drag.current[0] - event.clientX, drag.current[1] - event.clientY);
      }}>
      <div style={{ width: graph.width * zoom, height: graph.height * zoom, overflow: "hidden" }}>
        <div className="map-world" style={{ width: graph.width, height: graph.height, transform: `scale(${zoom})` }}>
          <svg aria-hidden="true" width={graph.width} height={graph.height}>
            {graph.edges.map(([a,b], i) => {
              const x = a.x + nodeWidth, y = a.y + 37, end = b.y + 37;
              const mid = x + (b.x - x) / 2;
              return <path key={i} d={`M ${x} ${y} C ${mid} ${y}, ${mid} ${end}, ${b.x} ${end}`} />;
            })}
          </svg>
          {graph.lanes.map(({worker, y}) => <Button key={worker} variant="secondary" className="map-agent" style={{ top: y }}
            aria-label={`View assignments for ${worker}`} onClick={() => select({worker})}><Bot />{worker}</Button>)}
          {graph.nodes.map(({task, x, y, order}) => <Button variant="outline" key={task.id} className="map-node" style={{ left: x, top: y, width: nodeWidth }}
            aria-label={`Milestone ${order + 1}: ${task.goal}`} onClick={() => select({worker: task.worker, task: task.id})}>
            <span><small>{order + 1}</small><Status state={task.state} /></span><strong>{task.goal}</strong>
          </Button>)}
        </div>
      </div>
    </div>
    {selection && <Dialog title={selectedTask?.id || selection.worker} onClose={() => select(null)}>
      {selection.coordinator ? <>
        <h3>Coordinator activity</h3><p>{coordinatorStatus(run)}</p>
        <ol className="turn-history">{(run.turns || []).filter(t => ["plan","observe","repair","chat"].includes(t.role)).map(t =>
          <li key={t.id}><span>{({plan:"Create plan",observe:"Review progress",repair:"Plan repair",chat:"Reply to message"})[t.role]}{t.agent && ` · ${t.agent}`}</span><Status state={t.state} /></li>)}</ol>
      </> : selectedTask ? <>
        <Button variant="outline" onClick={() => select({worker: selection.worker})}>All assignments for {selection.worker}</Button>
        <div><Status state={selectedTask.state} /></div>
        <Markdown>{selectedTask.goal}</Markdown>
        {!!selectedTask.depends?.length && <p>Depends on {selectedTask.depends.join(", ")}</p>}
        {selectedTask.parent && <p>Delegated by {selectedTask.parent}</p>}
        {selectedTask.finding && <Markdown>{selectedTask.finding}</Markdown>}
        {selectedTask.notes?.map((note, i) => <Markdown key={i}>{note}</Markdown>)}
      </> : <>
        <p>{[profile?.model || profile?.detected?.label || "Harness default", profile?.effort || profile?.detected?.effort].filter(Boolean).join(" · ")}</p>
        <ol className="agent-assignments">{tasks.map((task, i) => task.worker === selection.worker && <li key={task.id}>
          <Button variant="outline" onClick={() => select({worker: task.worker, task: task.id})}><span>{i + 1}. {task.goal}</span><Status state={task.state} /></Button>
          {!!task.depends?.length && <small>After {task.depends.join(", ")}</small>}
        </li>)}</ol>
        {!tasks.some(t => t.worker === selection.worker) && <p>No assignments yet.</p>}
      </>}
    </Dialog>}
  </Card>;
}
