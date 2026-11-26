import { Button } from "./components/ui/button";
import { Textarea } from "./components/ui/textarea";
import { Maximize2, Minimize2 } from "lucide-react";
import { AttachmentButton, FileList, addAttachments, encodeAttachments } from "./brief-files";
import React, { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { Command } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import { shortSummary } from "./run-summary.js";
import "./conversation.css";
import { coordinatorStatus } from "./status.jsx";

// Keep unsent drafts and reading positions scoped to the run and this open page.
const drafts = new Map();
const readingPositions = new Map();

function ChatIcon({ minimize = false }) {
  return <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
    strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d={minimize ? "m6 9 6 6 6-6" : "M20 11a8 8 0 0 1-8 8H8l-5 3 1.5-6A8.5 8.5 0 1 1 20 11Z"} />
  </svg>;
}

export function Proposal({ message, run, act, busy, onDecision }) {
  const [failed, setFailed] = useState(false);
  const canAdd = ["working", "review", "landed", "blocked", "checks"].includes(run.state) && (run.state !== "blocked" || run.recipe_approved) && !run.landing && !run.combination;
  async function decide(kind) {
    setFailed(false);
    if (await act({ kind, task: message.id })) onDecision?.();
    else setFailed(true);
  }

  return <section className="chat-proposal">
    <h3>Proposed milestones</h3>
    <ul>{message.tasks.map((task) => <li key={task.id}>
      <Markdown>{task.goal}</Markdown>
      <small>{task.worker}</small>{" "}
      {task.depends?.length > 0 && <small>After {task.depends.join(", ")}</small>}
    </li>)}</ul>
    {message.checks?.length > 0 && <>
      <h4>Additional checks</h4>
      <ul className="chat-checks">{message.checks.map((argv, index) => <li key={index}><Command argv={argv} /></li>)}</ul>
    </>}
    {message.proposal_state === "pending" ? <>
      <div className="chat-proposal-actions">
        <Button variant="outline" type="button" className="primary" disabled={busy || !canAdd} onClick={() => decide("accept-proposal")}>Add to run</Button>
        <Button variant="outline" type="button" disabled={busy} onClick={() => decide("dismiss-proposal")}>Dismiss proposal</Button>
      </div>
      {run.state === "landed" && <p>Adding these milestones starts a new round.</p>}
      {!canAdd && <p>Wait for the current run decision before adding milestones.</p>}
      {failed && <p role="alert">Couldn’t update the proposal. Try again.</p>}
    </> : <p className="proposal-outcome">{message.proposal_state === "added" ? "Milestones added" : "Proposal dismissed"}</p>}
  </section>;
}

function referenceLabel(run, reference) {
  const item = (reference.kind === "milestone" ? run.tasks : run.candidates)?.find((row) => row.id === reference.id);
  return shortSummary(item?.goal || item?.name || reference.id, 60);
}

function useLiveReply(api, runId, turnId, open) {
  const [preview, setPreview] = useState(null);
  useEffect(() => {
    if (!open || !turnId) return;
    const controller = new AbortController();
    let timer;
    async function read() {
      try {
        const text = await api(`/runs/${runId}/turns/${turnId}/reply`, undefined, controller.signal);
        if (!controller.signal.aborted) setPreview((current) => current?.turnId === turnId && current.text === text ? current : { turnId, text });
      } catch {
        // A later poll can recover; the run update carries the final answer or error.
      }
      if (!controller.signal.aborted) timer = setTimeout(read, 300);
    }
    read();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [api, runId, turnId, open]);
  return turnId && preview?.turnId === turnId ? preview.text : "";
}

export function Conversation({ run, act, busy, api, chatRequest, expanded, onExpand, onDock }) {
  const panelId = useId();
  const titleId = useId();
  const [dockOpen, setOpen] = useState(true);
  const open = expanded || dockOpen;
  const [files,setFiles] = useState([]);
  const [fileError,setFileError] = useState("");
  const activated = useRef(false);
  const [text, setText] = useState(() => drafts.get(run.id) || "");
  const [sendError, setSendError] = useState(false);
  const [sending, setSending] = useState(false);
  const [attachment, setAttachment] = useState(null);
  const explained = useRef(null);
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
  const pending = messages.find(({ state }) => state === "queued" || state === "running");
  const waiting = !!pending;
  const activeTurn = pending && run.turns?.findLast((turn) => turn.role === "chat" && turn.task === pending.id && turn.state === "running");
  const liveReply = useLiveReply(api, run.id, activeTurn?.id, open);
  const paused = run.state === "stopped" || (run.input.token_target > 0 && run.tokens >= run.input.token_target);
  const overLimit = new TextEncoder().encode(text.trim()).length > 6000;
  const full = messages.length >= 200;
  const available = !paused && !waiting && !busy && !sending && !full;
  const canSend = available && !overLimit && (!!text.trim() || files.length > 0) && !attachment;

  useEffect(() => {
    if (!chatRequest) return;
    readingPositions.delete(run.id);
    setAtLatest(true);
    setOpen(true);
    setAttachment(chatRequest);
    setSendError(false);
  }, [run.id, chatRequest]);

  async function askAbout(reference) {
    explained.current = reference.key;
    setSending(true);
    setSendError(false);
    const key = reference.kind === "milestone" ? "task" : "candidate";
    const ok = await act({ kind: "chat", text: `Explain this ${reference.kind}.`, [key]: reference.id });
    setSending(false);
    if (ok) setAttachment((current) => current?.key === reference.key ? null : current);
    else setSendError(true);
  }

  useEffect(() => {
    if (attachment && available && explained.current !== attachment.key) askAbout(attachment);
  }, [attachment, available]);

  useEffect(() => {
    if (open && atLatest) setSeenReply(lastReply);
  }, [open, atLatest, lastReply]);

  // Restore the reader's position on open. Follow replies only while at the bottom.
  useLayoutEffect(() => {
    if (!open) return;
    const log = history.current;
    log.scrollTop = readingPositions.get(run.id) ?? log.scrollHeight;
  }, [open, expanded, run.id, messages.length, waiting, liveReply, chatRequest]);

  function trackPosition() {
    const log = history.current;
    const nearEnd = log.scrollHeight - log.clientHeight - log.scrollTop < 24;
    if (nearEnd) readingPositions.delete(run.id);
    else readingPositions.set(run.id, log.scrollTop);
    setAtLatest(nearEnd);
  }

  useLayoutEffect(() => {
    if (open && activated.current) {
      const target = window.matchMedia("(pointer: coarse)").matches ? minimize : composer;
      target.current.focus({ preventScroll: true });
    } else if (restoreFocus.current) {
      launcher.current.focus({ preventScroll: true });
      restoreFocus.current = false;
    }
  }, [open, expanded]);

  function close() {
    if (expanded) { setOpen(true); onDock(); return; }
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
    setSending(true);
    let ok;
    try {
      const attachments = await encodeAttachments(files);
      ok = await act({ kind: "chat", text: text.trim() || "Review the attached files.", ...(attachments.length ? {attachments} : {}) });
    } catch (error) { setFileError(error.message); }
    if (ok) { setFiles([]); setFileError(""); }
    setSending(false);
    if (!ok) {
      setSendError(true);
      return;
    }
    if (drafts.get(run.id) === text) edit("");
    composer.current?.focus({ preventScroll: true });
  }

  return <div className={expanded ? "conversation-page" : "coordinator-chat"}>
    <Button variant="outline" ref={launcher} type="button" className="chat-launcher" aria-label="Coordinator chat"
      aria-description={unread ? "New reply" : undefined} aria-expanded={open} aria-controls={panelId}
      aria-haspopup="dialog" hidden={open} onClick={() => { activated.current = true; setOpen(true); }}>
      <ChatIcon />Coordinator chat{unread && <span className="chat-unread" aria-hidden="true" />}
    </Button>
    <div id={panelId}>{open && <section role={expanded ? "region" : "dialog"} aria-labelledby={titleId} className="chat-panel"
      onKeyDown={(event) => {
        if (event.key === "Escape") { event.stopPropagation(); close(); }
      }}>
      <header className="chat-header"><div><h2 id={titleId}>Coordinator chat</h2>
        <span>{run.input.coordinator}</span></div>
        <div className="chat-window-actions">{!expanded && <Button variant="ghost" size="icon" aria-label="Full screen coordinator chat" title="Full screen" onClick={() => {activated.current=true; onExpand();}}><Maximize2/></Button>}
        <Button variant="ghost" ref={minimize} type="button" title={expanded ? "Return to dock" : "Minimize chat"} aria-label={expanded ? "Return to dock" : "Minimize chat"} onClick={close}>{expanded ? <Minimize2/> : <ChatIcon minimize />}</Button></div>
      </header>
      <div className="coordinator-status" role="status">{coordinatorStatus(run)}</div>
      <div ref={history} className="chat-history" role="log" aria-label="Coordinator conversation" tabIndex={0}
        onScroll={trackPosition}>
        {messages.length === 0 && <p className="chat-empty">Ask about this run or request another milestone.</p>}
        {messages.map((message) => <article className={`chat-message${message.role === "user" ? " from-user" : ""}`} key={message.id}>
          <strong className="chat-author">{message.role === "user" ? "You" : "Coordinator"}</strong>
          {message.context && <small className="chat-context">{referenceLabel(run, message.context)}</small>}
          <Markdown>{message.text}</Markdown>
          {!!message.attachments?.length && <ul aria-label="Attached files">{message.attachments.map(f=><li key={f.path}>{f.name}</li>)}</ul>}
          {message.error && <p className="chat-error" role="alert">{message.error}</p>}
          {message.tasks?.length > 0 && <Proposal {...{ message, run, act, busy }} />}
        </article>)}
        {waiting && <article className="chat-message chat-live" aria-live="off">
          <strong className="chat-author">Coordinator</strong>
          {liveReply && <Markdown>{liveReply}</Markdown>}
          <span className="chat-progress" role="status"><i aria-hidden="true" />{liveReply ? "Writing…" : "Waiting for the coordinator…"}</span>
        </article>}
      </div>
      {!atLatest && <Button variant="outline" className="chat-latest" type="button" onClick={() => {
        history.current.scrollTop = history.current.scrollHeight;
        trackPosition();
      }}>Latest messages ↓</Button>}
      <form className="chat-composer" onSubmit={send}>
        {attachment && <div className="chat-attachment">
          <span>Explain {attachment.kind} · {referenceLabel(run, attachment)}</span>
          {sendError && <Button variant="outline" type="button" disabled={!available} onClick={() => askAbout(attachment)}>Retry</Button>}
          {!sending && <Button variant="outline" type="button" aria-label="Cancel explanation" onClick={() => { setAttachment(null); setSendError(false); }}>×</Button>}
        </div>}
        {expanded && <AttachmentButton busy={sending || paused} label="Chat attachments" add={picked=>{
          try { setFiles(addAttachments(files,picked)); setFileError(""); } catch (error) {setFileError(error.message);}
        }}/>}
        <FileList names={files.map(f=>f.name)} disabled={sending} remove={index=>setFiles(files.filter((_,i)=>i!==index))}/>
        {fileError && <p role="alert">{fileError}</p>}
        {sending && <p role="status">Sending…</p>}
        {paused && <p>Resume the run to send a message.</p>}
        {full && <p>This conversation is full.</p>}
        {overLimit && <p role="alert">Shorten your message to 6,000 bytes.</p>}
        {sendError && <p className="chat-error" role="alert">Your message wasn’t sent. Try again.</p>}
        <div>
          <Textarea ref={composer} aria-label="Message" placeholder="Message the coordinator…" rows={2}
            value={text} onChange={(event) => edit(event.target.value)} onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229) {
                event.preventDefault();
                event.currentTarget.form.requestSubmit();
              }
            }} />
          <Button variant="outline" type="submit" className="primary" title="Enter to send · Shift+Enter for a new line" disabled={!canSend}>Send</Button>
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
