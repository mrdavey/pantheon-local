import React, { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Command } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import "./conversation.css";

// Keep unsent drafts and reading positions scoped to the run and this open page.
const drafts = new Map();
const readingPositions = new Map();

function ChatIcon({ minimize = false }) {
  return <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
    strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d={minimize ? "m6 9 6 6 6-6" : "M20 11a8 8 0 0 1-8 8H8l-5 3 1.5-6A8.5 8.5 0 1 1 20 11Z"} />
  </svg>;
}

function Proposal({ message, run, act, busy }) {
  const [failed, setFailed] = useState(false);
  const canAdd = ["working", "review"].includes(run.state) && !run.landing;
  async function decide(kind) {
    setFailed(false);
    setFailed(!await act({ kind, task: message.id }));
  }

  return <section className="chat-proposal">
    <h3>Proposed milestones</h3>
    <ul>{message.tasks.map((task) => <li key={task.id}>
      <Markdown>{task.goal}</Markdown>
      {task.depends?.length > 0 && <small>After {task.depends.join(", ")}</small>}
    </li>)}</ul>
    {message.checks?.length > 0 && <>
      <h4>Additional checks</h4>
      <ul className="chat-checks">{message.checks.map((argv, index) => <li key={index}><Command argv={argv} /></li>)}</ul>
    </>}
    {message.proposal_state === "pending" ? <>
      <div className="chat-proposal-actions">
        <button type="button" className="primary" disabled={busy || !canAdd} onClick={() => decide("accept-proposal")}>Add milestones</button>
        <button type="button" disabled={busy} onClick={() => decide("dismiss-proposal")}>Dismiss proposal</button>
      </div>
      {!canAdd && <p>{run.state === "landed" ? "Open Experiment to start another round." : "You can add milestones when the run is working or ready for review."}</p>}
      {failed && <p role="alert">Couldn’t update the proposal. Try again.</p>}
    </> : <p className="proposal-outcome">{message.proposal_state === "added" ? "Milestones added" : "Proposal dismissed"}</p>}
  </section>;
}

export function Conversation({ run, act, busy }) {
  const panelId = useId();
  const titleId = useId();
  const [open, setOpen] = useState(false);
  const [text, setText] = useState(() => drafts.get(run.id) || "");
  const [sendError, setSendError] = useState(false);
  const [atLatest, setAtLatest] = useState(() => !readingPositions.has(run.id));
  const launcher = useRef(null);
  const minimize = useRef(null);
  const restoreFocus = useRef(false);
  const composer = useRef(null);
  const history = useRef(null);
  const messages = run.conversation || [];
  const lastReply = messages.filter(({ role }) => role === "coordinator").at(-1)?.id;
  const [seenReply, setSeenReply] = useState(lastReply);
  const unread = lastReply && lastReply !== seenReply;
  const waiting = messages.some(({ state }) => state === "queued" || state === "running");
  const paused = run.state === "stopped" || (run.input.token_target > 0 && run.tokens >= run.input.token_target);
  const overLimit = new TextEncoder().encode(text.trim()).length > 6000;
  const full = messages.length >= 200;
  const canSend = !paused && !waiting && !busy && !overLimit && !full && !!text.trim();

  useEffect(() => {
    if (open && atLatest) setSeenReply(lastReply);
  }, [open, atLatest, lastReply]);

  // Restore the reader's position on open. Follow replies only while at the bottom.
  useLayoutEffect(() => {
    if (!open) return;
    const log = history.current;
    log.scrollTop = readingPositions.get(run.id) ?? log.scrollHeight;
  }, [open, run.id, messages.length, waiting]);

  function trackPosition() {
    const log = history.current;
    const nearEnd = log.scrollHeight - log.clientHeight - log.scrollTop < 24;
    if (nearEnd) readingPositions.delete(run.id);
    else readingPositions.set(run.id, log.scrollTop);
    setAtLatest(nearEnd);
  }

  useLayoutEffect(() => {
    if (open) {
      const target = window.matchMedia("(pointer: coarse)").matches ? minimize : composer;
      target.current.focus({ preventScroll: true });
    } else if (restoreFocus.current) {
      launcher.current.focus({ preventScroll: true });
      restoreFocus.current = false;
    }
  }, [open]);

  function close() {
    restoreFocus.current = true;
    setOpen(false);
  }

  function edit(value) {
    if (value) drafts.set(run.id, value);
    else drafts.delete(run.id);
    setText(value);
    setSendError(false);
  }

  async function send(event) {
    event.preventDefault();
    if (!canSend) return;
    if (!await act({ kind: "chat", text })) {
      setSendError(true);
      return;
    }
    if (drafts.get(run.id) === text) edit("");
    composer.current?.focus({ preventScroll: true });
  }

  return <div className="coordinator-chat">
    <button ref={launcher} type="button" className="chat-launcher" aria-label="Coordinator chat"
      aria-description={unread ? "New reply" : undefined} aria-expanded={open} aria-controls={panelId}
      aria-haspopup="dialog" hidden={open} onClick={() => setOpen(true)}>
      <ChatIcon />Coordinator chat{unread && <span className="chat-unread" aria-hidden="true" />}
    </button>
    <div id={panelId}>{open && <section role="dialog" aria-labelledby={titleId} className="chat-panel"
      onKeyDown={(event) => {
        if (event.key === "Escape") { event.stopPropagation(); close(); }
      }}>
      <header className="chat-header"><div><h2 id={titleId}>Coordinator chat</h2>
        <span>{run.input.coordinator}</span></div>
        <button ref={minimize} type="button" title="Minimize chat" aria-label="Minimize chat" onClick={close}><ChatIcon minimize /></button>
      </header>
      <div ref={history} className="chat-history" role="log" aria-label="Coordinator conversation" tabIndex={0}
        onScroll={trackPosition}>
        {messages.length === 0 && <p className="chat-empty">Ask about this run or request another milestone.</p>}
        {messages.map((message) => <article className={`chat-message${message.role === "user" ? " from-user" : ""}`} key={message.id}>
          <strong className="chat-author">{message.role === "user" ? "You" : "Coordinator"}</strong>
          <Markdown>{message.text}</Markdown>
          {message.error && <p className="chat-error" role="alert">{message.error}</p>}
          {message.tasks?.length > 0 && <Proposal {...{ message, run, act, busy }} />}
        </article>)}
      </div>
      {!atLatest && <button className="chat-latest" type="button" onClick={() => {
        history.current.scrollTop = history.current.scrollHeight;
        trackPosition();
      }}>Latest messages ↓</button>}
      <form className="chat-composer" onSubmit={send}>
        {waiting && <p role="status">Waiting for the coordinator…</p>}
        {paused && <p>Resume the run to send a message.</p>}
        {full && <p>This conversation is full.</p>}
        {overLimit && <p role="alert">Shorten your message to 6,000 bytes.</p>}
        {sendError && <p className="chat-error" role="alert">Your message wasn’t sent. Try again.</p>}
        <div>
          <textarea ref={composer} aria-label="Message" placeholder="Message the coordinator…" rows={2}
            value={text} onChange={(event) => edit(event.target.value)} onKeyDown={(event) => {
              if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !event.nativeEvent.isComposing) {
                event.preventDefault();
                event.currentTarget.form.requestSubmit();
              }
            }} />
          <button type="submit" className="primary" title="Send (⌘/Ctrl + Enter)" disabled={!canSend}>Send</button>
        </div>
      </form>
    </section>}</div>
  </div>;
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
