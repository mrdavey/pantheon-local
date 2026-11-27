import React, { useEffect, useRef, useState } from "react";
import { ArrowLeft, Bot, CornerDownRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { Label } from "./components/ui/label";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { activityRoles } from "./activity-log";
import { changeRoute } from "./navigation";
import { shortSummary } from "./run-summary";
import { Loading, Status } from "./status";
import { TurnLog } from "./turn-log";
import "./run-agents.css";

function groupBy(items, key) {
  const groups = new Map();
  for (const item of items) {
    const group = key(item);
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(item);
  }
  return groups;
}

function latest(turns) {
  return turns.findLast(turn => turn.state === "running") || turns.at(-1);
}

function roster(run, children) {
  const coordinator = run.input.coordinator;
  const tasks = run.tasks || [];
  const turns = (run.turns || []).filter(turn => turn.agent);
  const workerTurns = turns.filter(turn => turn.role === "worker");
  const coordinatorTurns = groupBy(turns.filter(turn => turn.role !== "worker"), turn => turn.agent);
  const coordinators = [...new Set([coordinator, ...coordinatorTurns.keys()].filter(Boolean))];
  const workers = [...new Set([...(run.input.workers || []), ...tasks.map(task => task.worker),
    ...workerTurns.map(turn => turn.agent)].filter(Boolean))];
  const assignments = new Map(tasks.map(task => [task.id, task]));
  const owned = groupBy(tasks, task => assignments.has(task.parent) ? task.parent : null);
  const attempts = groupBy(workerTurns, turn => turn.task);
  const native = groupBy(children, child => `${child.turn}:${child.parent || ""}`);

  function nativeRows(turn, parent = "") {
    return (native.get(`${turn.id}:${parent}`) || []).map(child => ({
      key: `subagent:${turn.id}:${child.id}`, title: child.title, detail: child.detail,
      state: child.state, turns: [turn], child, delegated: true, children: nativeRows(turn, child.id),
    }));
  }

  function assignment(task) {
    const turns = attempts.get(task.id) || [];
    return {
      key: `milestone:${task.id}`, title: shortSummary(task.goal) || task.id, detail: task.worker,
      state: task.state, turns, task: task.id, delegated: Boolean(task.parent),
      children: [...(owned.get(task.id) || []).map(assignment), ...turns.flatMap(turn => nativeRows(turn))],
    };
  }

  const workerRows = workers.map(name => {
    const turns = workerTurns.filter(turn => turn.agent === name);
    const active = latest(turns);
    const current = active && assignments.get(active.task);
    const rootTasks = (owned.get(null) || []).filter(task => task.worker === name);
    const unlinked = turns.filter(turn => !assignments.has(turn.task));
    return {
      key: `worker:${name}`, title: name, detail: current ? shortSummary(current.goal, 60) : "Worker",
      model: run.profiles?.[name]?.model || run.profiles?.[name]?.detected?.label,
      state: current?.state || active?.state || rootTasks[0]?.state || "idle",
      turns, children: [...rootTasks.map(assignment), ...unlinked.flatMap(turn => nativeRows(turn))],
    };
  });
  const coordinatorRows = coordinators.map(name => {
    const turns = coordinatorTurns.get(name) || [];
    const current = name === coordinator;
    return {
      key: `coordinator:${name}`, title: current ? "Coordinator" : name, detail: current ? name : "Former coordinator",
      turns, model: run.profiles?.[name]?.model || run.profiles?.[name]?.detected?.label,
      state: latest(turns)?.state || (turns.length ? "received" : "idle"),
      children: turns.flatMap(turn => nativeRows(turn)),
    };
  });
  return [...coordinatorRows, ...workerRows];
}

function findAgent(rows, key) {
  for (const row of rows) {
    if (row.key === key) return row;
    const child = findAgent(row.children, key);
    if (child) return child;
  }
}

function AgentRows({ rows, selected, select }) {
  return <ul className="agent-rows">
    {rows.map(row => <li key={row.key}>
      <Button variant="ghost" className="agent-row" data-agent-key={row.key}
        aria-current={selected === row.key ? "true" : undefined}
        onClick={() => select(row.key)}>
        {row.delegated ? <CornerDownRight aria-hidden="true" /> : !row.task && <Bot aria-hidden="true" />}
        <span className="agent-row-text">
          <strong title={row.title}>{row.title}</strong>
          {row.detail && <small title={row.detail}>{row.detail}</small>}
        </span>
        <Status state={row.state} />
      </Button>
      {row.children.length > 0 && <AgentRows rows={row.children} {...{selected, select}} />}
    </li>)}
  </ul>;
}

function turnTitle(run, turn) {
  if (turn.role === "worker") {
    return shortSummary(run.tasks?.find(task => task.id === turn.task)?.goal) || turn.task || "Worker";
  }
  return activityRoles[turn.role] || turn.role;
}

export function RunAgents({ run, route, api, openMilestone }) {
  const [native, setNative] = useState(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const active = run.turns?.some(turn => turn.state === "running" && turn.agent) || false;
  const heading = useRef(null);
  const list = useRef(null);
  const focusTarget = useRef(null);

  useEffect(() => {
    const controller = new AbortController();
    let timer;
    async function read() {
      try {
        const result = await api(`/runs/${run.id}/agents`, undefined, controller.signal);
        if (!controller.signal.aborted) {
          setNative(result);
          setError("");
        }
      } catch (problem) {
        if (!controller.signal.aborted) setError(problem.message);
      }
      if (active && !controller.signal.aborted) timer = setTimeout(read, 2000);
    }
    setError("");
    read();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [run.id, run.revision, api, active, retry]);

  const rows = roster(run, native || []);
  const selected = findAgent(rows, route.agent);
  const turn = selected?.turns.find(turn => route.agentTurn === turn.id) || (selected && latest(selected.turns));
  const view = route.agentView === "raw" ? "raw" : "output";
  const showDetail = Boolean(selected || route.agent);
  useEffect(() => {
    const target = focusTarget.current;
    if (selected && target === "detail") heading.current?.focus();
    if (!showDetail && target && target !== "detail") {
      const row = Array.from(list.current?.querySelectorAll("[data-agent-key]") || [])
        .find(element => element.dataset.agentKey === target);
      row?.focus();
    }
    if (target) focusTarget.current = null;
  }, [selected?.key, showDetail]);

  function select(key) {
    if (key === route.agent) return;
    focusTarget.current = key ? "detail" : route.agent;
    changeRoute({agent: key, agentTurn: null, agentView: null});
  }

  return <section className="run-agents" data-detail={showDetail} aria-label="Run agents">
    <div className="agent-sidebar" ref={list}>
      <h2>Agents</h2>
      <nav aria-label="Agent roster"><AgentRows rows={rows} selected={selected?.key} select={select} /></nav>
      {!native && !error && <Loading>Loading subagents…</Loading>}
      {error && <p className="agent-error" role="alert">{error}{" "}
        <Button size="sm" variant="outline" onClick={() => setRetry(value => value + 1)}>Try again</Button>
      </p>}
    </div>
    <div className="agent-detail">
      {showDetail && <Button variant="ghost" className="agent-back" onClick={() => select(null)}>
        <ArrowLeft aria-hidden="true" />Back to agents
      </Button>}
      {selected ? <>
        <header className="agent-detail-heading">
          <div>
            <h2 ref={heading} tabIndex={-1}>{selected.title}</h2>
            {(selected.detail || selected.model) && <p>{[selected.detail, selected.model].filter(Boolean).join(" · ")}</p>}
          </div>
          <Status state={selected.state} />
        </header>
        {selected.task && <Button variant="outline" size="sm" onClick={() => openMilestone(selected.task)}>View milestone</Button>}
        {turn ? <>
          <div className="agent-log-toolbar">
            {selected.turns.length > 1 && <Label className="agent-turn-choice">Turn
              <NativeSelect value={turn.id} onChange={event => changeRoute({agentTurn: event.target.value})}>
                {[...selected.turns].reverse().map((entry, index) => <NativeSelectOption key={entry.id} value={entry.id}>
                  {selected.task ? `Attempt ${selected.turns.length - index}` : turnTitle(run, entry)}
                  {" · "}{entry.started ? new Date(entry.started).toLocaleString() : `Turn ${selected.turns.length - index}`}
                </NativeSelectOption>)}
              </NativeSelect>
            </Label>}
            <div className="agent-log-views" role="group" aria-label="Agent log view">
              <Button size="sm" variant={view === "output" ? "secondary" : "ghost"} aria-pressed={view === "output"}
                onClick={() => changeRoute({agentView: null, agentTurn: turn.id})}>Output</Button>
              <Button size="sm" variant={view === "raw" ? "secondary" : "ghost"} aria-pressed={view === "raw"}
                onClick={() => changeRoute({agentView: "raw", agentTurn: turn.id})}>Raw log</Button>
            </div>
          </div>
          {selected.child && !selected.child.transcript && <p className="agent-empty">Transcript unavailable.</p>}
          <TurnLog key={`${turn.id}:${selected.child?.id || ""}:${view}`} {...{api, view}}
            unavailable={Boolean(selected.child && !selected.child.transcript)} child={selected.child?.id} path={`/runs/${run.id}/turns/${turn.id}/log`}
            running={turn.state === "running"} />
        </> : <p className="agent-empty">No turns yet.</p>}
      </> : <>
        <p className="agent-empty">
          {route.agent ? (error ? "Could not load this agent." : native ? "This agent is unavailable." : "Loading agent…") : "Select an agent to view its output."}
        </p>
        {route.agent && error && <Button className="agent-back" variant="outline" onClick={() => setRetry(value => value + 1)}>Try again</Button>}
      </>}
    </div>
  </section>;
}
