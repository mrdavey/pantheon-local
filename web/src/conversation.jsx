import React, { useState } from "react";

export function Conversation({ run, act, busy }) {
  const [text, setText] = useState("");
  const messages = run.conversation || [];
  const waiting = messages.some(({ state }) => state === "queued" || state === "running");
  const canAdd = ["working", "review"].includes(run.state);
  const paused = run.state === "stopped" || (run.input.token_target > 0 && run.tokens >= run.input.token_target);
  async function send(event) {
    event.preventDefault();
    if (await act({ kind: "chat", text })) setText("");
  }

  return <section className="conversation" aria-labelledby="conversation-title">
    <h2 id="conversation-title">Talk to the coordinator</h2>
    <div role="log" aria-label="Coordinator conversation">
      {messages.length === 0 && <p>Ask about this run or request another milestone.</p>}
      {messages.map((message) => <article className="row" key={message.id}>
        <strong>{message.role === "user" ? "You" : "Coordinator"}</strong>
        <p className="message-text">{message.text}</p>
        {message.error && <p role="alert">{message.error}</p>}
        {message.tasks?.length > 0 && <>
          <h3>Proposed milestones</h3>
          <ul className="proposal-list">{message.tasks.map((task) => <li key={task.id}><strong>{task.id}</strong> {task.goal}{task.depends?.length > 0 && <small>After {task.depends.join(", ")}</small>}</li>)}</ul>
          {message.checks?.length > 0 && <details><summary>Additional checks</summary><pre>{JSON.stringify(message.checks, null, 2)}</pre></details>}
          {message.proposal_state === "pending" ? <div className="actions">
            <button className="primary" disabled={busy || !canAdd} onClick={() => act({ kind: "accept-proposal", task: message.id })}>Add milestones</button>
            <button disabled={busy} onClick={() => act({ kind: "dismiss-proposal", task: message.id })}>Dismiss proposal</button>
          </div> : <p>{message.proposal_state === "added" ? "Milestones added" : "Proposal dismissed"}</p>}
          {message.proposal_state === "pending" && !canAdd && <p>{run.state === "landed" ? "Open Experiment to start another round." : "You can add these milestones when work resumes or the candidate is ready for review."}</p>}
        </>}
      </article>)}
    </div>
    {waiting && <p role="status">Waiting for the coordinator…</p>}
    {paused && <p>Resume the run to send a message.</p>}
    <form onSubmit={send}>
      <label>Message<textarea rows={3} value={text} onChange={(event) => setText(event.target.value)} /></label>
      <button className="primary" type="submit" disabled={paused || waiting || busy || !text.trim()}>Send</button>
    </form>
  </section>;
}

export function RoutingEvidence({ route }) {
  if (!route) return null;
  const evidence = route.evidence;
  return <details className="routing-evidence">
    <summary>{evidence ? "Why this model?" : "Why this worker?"}</summary>
    <p>{route.reason} {evidence && <>Compared {route.compared} profiles.</>}</p>
    {evidence && <p>{evidence.model} · {evidence.agent}{evidence.effort && " · " + evidence.effort} · {evidence.score.toFixed(1)}%{evidence.cost_per_trial != null && " · $" + evidence.cost_per_trial.toFixed(2) + " per benchmark trial"}. <a href={evidence.source} target="_blank" rel="noreferrer">{evidence.suite}, {evidence.date}</a></p>}
  </details>;
}
