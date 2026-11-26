import { Card } from "./components/ui/card";
import { Disclosure } from "./disclosure";
import { Alert } from "./components/ui/alert";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Textarea } from "./components/ui/textarea";
import { Label } from "./components/ui/label";
import { Status, Loading, LoadingProvider } from "./status.jsx";
import { WorkbenchHeader, RunFooter } from "./workbench-shell";
import { ListTree, Activity, ChartNoAxesCombined, FlaskConical, MessageSquare, Play } from "lucide-react";
import { ActivityLog } from "./activity-log";
import { Milestones } from "./milestones";
import { Metrics } from "./metrics";
import { ImplementationDetails, implementationLabel } from "./implementation.jsx";
import { Dialog } from "./dialog.jsx";
import React, { useEffect, useRef, useState } from "react";
import { RunSettings } from "./run-settings.jsx";
import { createRoot } from "react-dom/client";
import "./style.css";
import "./work.css";
import { DiffExplorer, TurnOutput } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import { runTitle, shortSummary, blockedTasks } from "./run-summary.js";
import { AgentSetup } from "./setup.jsx";
import { Brief } from "./brief.jsx";
import { Experiment, Inbox, ReleaseList } from "./experiments.jsx";
import { Conversation } from "./conversation.jsx";
import { RecipeEditor, branchName } from "./review.jsx";

import "./workspace.css";
import "./workbench-shell.css";

const tokenKey = "local-swarm-token";
const [launchToken, ...launchOptions] = location.hash.slice(1).split("&");
const token = launchToken || sessionStorage.getItem(tokenKey);
const cwdKey = "local-swarm-launch-folder";
if (launchToken) {
  const cwd = new URLSearchParams(launchOptions.join("&")).get("cwd");
  if (cwd) sessionStorage.setItem(cwdKey, cwd);
  else sessionStorage.removeItem(cwdKey);
}
if (location.hash && token) {
  sessionStorage.setItem(tokenKey, token);
  history.replaceState(null, "", location.pathname);
}

// A terminal link can open in a tab already showing the same loopback address.
window.addEventListener("hashchange", () => {
  if (/^#[a-f0-9]{64}(?:&|$)/.test(location.hash)) location.reload();
});

async function api(path, body, signal) {
  const response = await fetch("/api" + path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    const detail = await response.json().catch(() => null);
    const problem = new Error(response.status === 401 ? "Open the workbench link in Terminal. On the Pantheon screen, you can also press Enter." : detail?.error || response.statusText || `Request failed (${response.status}).`);
    problem.status = response.status;
    throw problem;
  }
  return response.json();
}

const iconPaths = {
  brand: "M3 19h18M5 16V9m5 7V5m5 11V5m4 11V9M3 9l9-6 9 6",
  plus: "M12 5v14M5 12h14",
  agents: "M8 8a4 4 0 1 0 8 0 4 4 0 0 0-8 0ZM4 21v-2a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v2",
  inbox: "M5 5h14l3 9v5H2v-5l3-9Zm-3 9h6l2 2h4l2-2h6",
  settings: "M9.2 4.5 10 2h4l.8 2.5 2 .9 2.5-.4 2 3.5-1.7 1.9v3.2l1.7 1.9-2 3.5-2.5-.4-2 .9L14 22h-4l-.8-2.5-2-.9-2.5.4-2-3.5 1.7-1.9v-3.2L2.7 8.5l2-3.5 2.5.4 2-.9ZM15.5 12a3.5 3.5 0 1 0-7 0 3.5 3.5 0 0 0 7 0Z",
  folder: "M3 7h7l2 2h9v10H3V7Z",
};

function Icon({ name }) {
  return <svg className="icon" width="20" height="20" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d={iconPaths[name]} />
  </svg>;
}

function Question({ text, onAnswer, busy, submitLabel = "Send answer" }) {
  const [answer, setAnswer] = useState("");
  return <form className="answer" onSubmit={(event) => { event.preventDefault(); onAnswer(answer); }}>
    <Markdown>{text}</Markdown>
    <Textarea aria-label={text} required rows={2} value={answer} onChange={(event) => setAnswer(event.target.value)} />
    <Button variant="default" className="primary" type="submit" disabled={busy || !answer.trim()}>{submitLabel}</Button>
  </form>;
}

function Evidence({ path, kind, name, refreshable }) {
  const [text, setText] = useState(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  useEffect(() => {
    const request = new AbortController();
    setError("");
    setLoading(true);
    api(path, undefined, request.signal).then((value) => {
      if (!request.signal.aborted) setText(value);
    }).catch((problem) => {
      if (!request.signal.aborted) setError(problem.message);
    }).finally(() => {
      if (!request.signal.aborted) setLoading(false);
    });
    return () => request.abort();
  }, [path, reload]);
  return <>
    {error && <p className="notice error" role="alert">{error}</p>}
    {loading && <Loading>{kind === "diff" ? "Loading changes…" : "Loading output…"}</Loading>}
    {text !== null && (kind === "diff" ? <DiffExplorer text={text} /> : <TurnOutput name={name} source={text} />)}
    {(error || refreshable) && <Button variant="outline" disabled={loading} onClick={() => setReload(reload + 1)}>{error ? "Try again" : "Refresh log"}</Button>}
  </>;
}

function Candidate({ run, candidate, act, perform, busy, name, explain }) {
  const [preview, setPreview] = useState("");
  const [evidence, setEvidence] = useState(null);
  const evalTurn = (run.turns || []).findLast((turn) => turn.role === "eval" && (turn.task === candidate.id || turn.id === candidate.evidence));
  const [details, setDetails] = useState(false);
  const [changing, setChanging] = useState(false);
  const [target, setTarget] = useState("");
  const needsSelection = !!run.input.recipe.rubric && run.selection?.candidate !== candidate.id;
  const canLand = !candidate.baseline && candidate.eligible !== false && (candidate.policy || 0) === (run.policy || 0) &&
    (candidate.round || 0) === (run.round || 0) && candidate.state === "passed" && run.state === "review";
  const previewCommand = candidate.round ? candidate.preview : run.input.recipe.preview;
  const blocked = (run.inbox || []).some((q) => q.state === "open" && q.blocking &&
    (!q.tasks?.length || q.tasks.some((task) => candidate.tasks?.includes(task))));
  const load = (kind, path) => setEvidence({ kind, path: "/runs/" + run.id + path });
  return <Card asChild><article className="row work-result">
    <header>{name ? <h3>{name}</h3> : <h2>Implementation</h2>}<Status state={candidate.state} /></header>
    {candidate.tasks?.length > 0 && <p>{candidate.tasks.length} {candidate.tasks.length === 1 ? "milestone" : "milestones"} included</p>}
    {candidate.detail && <p>{shortSummary(candidate.detail)}</p>}
    {canLand && <>
      {!run.repo.Target && <Label>Target branch<Input value={target} onChange={(event) => setTarget(event.target.value)} /></Label>}
      {needsSelection && <p>Select this implementation in Experiment after reviewing the criteria.</p>}
      {blocked && <p>Answer the blocking questions before landing this implementation.</p>}
      <div className="actions">
        <Button variant="default" className="primary" disabled={busy || blocked || needsSelection || !!run.combination || (!run.repo.Target && !target.trim())}
          onClick={() => act({ kind: "land", candidate: candidate.id, text: target })}>
          {run.repo.Target ? `Land on ${branchName(run.repo.Target)}` : "Land locally"}
        </Button>
        <Button variant="outline" disabled={busy || !!run.combination} onClick={() => setChanging(true)}>Request change</Button>
      </div>
      {changing && <Dialog title="Request a change" onClose={() => setChanging(false)}>
        <Question text="What should change?" submitLabel="Request revision" busy={busy} onAnswer={async (text) => {
          if (await act({ kind: "change", candidate: candidate.id, text })) setChanging(false);
        }} /></Dialog>}
    </>}
    <div className="actions result-tools">
      {candidate.commit && <Button variant="outline" disabled={busy} onClick={() => load("diff", "/diff/" + candidate.id)}>View diff</Button>}
      {evalTurn && <Button variant="outline" disabled={busy} onClick={() => load("checks", "/turns/" + evalTurn.id)}>View checks</Button>}
      {candidate.commit && <Button variant="outline" disabled={busy} onClick={() => perform(() => api("/runs/" + run.id + "/open-folder", { candidate: candidate.id }), "Opening folder…")}>Open output folder</Button>}
      {candidate.commit && previewCommand?.length > 0 && !preview && <Button variant="outline" disabled={busy || candidate.state === "evaluating"} onClick={() => perform(async () => setPreview((await api("/runs/" + run.id + "/preview/" + candidate.id, {})).url), "Starting preview…")}>Start preview</Button>}
      <Button variant="outline" onClick={() => setDetails(true)}>View details</Button>
      {preview && <a href={preview} target="_blank" rel="noreferrer">Open preview ↗</a>}
    </div>
    {details && <ImplementationDetails {...{ run, candidate, explain }} onClose={() => setDetails(false)} />}
    {evidence && <Dialog title={evidence.kind === "diff" ? "File changes" : "Acceptance checks"} className="evidence-dialog" onClose={() => setEvidence(null)}>
      <Evidence {...evidence} name="Acceptance checks" refreshable={evidence.kind === "checks" && evalTurn?.state === "running"} />
    </Dialog>}
  </article></Card>;
}

function RunView({ run, send, perform, busy, openRun, openInbox, settings }) {
  const [view, setTab] = useState(() => run.selection && (run.input.compare || ["auto", "deploy"].includes(run.input.delivery)) ? "experiment" : "work");
  const [experimentOpened, setExperimentOpened] = useState(view === "experiment");
  const lastView = useRef(view);
  const [selectedTask,setSelectedTask] = useState(null);
  const [activityTurn,setActivityTurn] = useState(null);
  const openMilestone = id => { setTab("work"); setSelectedTask(id); };
  const openActivity = row => { setActivityTurn(row); setTab("activity"); };
  const expandChat = () => { if (view !== "chat") lastView.current = view; setTab("chat"); };
  const [chatRequest, setChatRequest] = useState(null);
  const explain = (kind, id) => {
    setChatRequest({kind, id, key: crypto.randomUUID()});
  };
  const act = (body) => send(run, body);
  const questions = (run.inbox || []).filter((q) => q.state === "open" && (q.source !== "plan" || run.state !== "question"));
  const landed = run.state === "landed" && run.detail;
  const showExperiment = () => { setExperimentOpened(true); setTab("experiment"); };
  const tasks = run.tasks || [];
  const candidates = (run.candidates || []).filter(c => (c.round || 1) === (run.round || 1));
  const activeCandidates = candidates.filter(c => (c.policy || 0) === (run.policy || 0) && c.state !== "superseded");
  const superseded = candidates.filter(c => c.state === "superseded");
  const renderCandidate = (candidate) => <Candidate key={candidate.id} name={activeCandidates.length === 1 && candidate === activeCandidates[0] ? "" : implementationLabel(candidate)}
    {...{ candidate, run, act, perform, busy, explain }} />;
  const subdir = run.repo.Subdir;
  const projectPath = subdir && subdir !== "." ? run.repo.Root.replace(/\/$/, "") + "/" + subdir : run.repo.Root;
  return <section className="panel run-workspace">
    <header className="run-heading">
      <div className="run-crumb">Runs <span>/</span> {run.id.slice(0,8)}</div>
      <div className="run-title"><h1 tabIndex={-1}>{runTitle(run)}</h1><Status state={run.state} run={run}/></div>
      <div className="run-meta">
        <Button variant="outline" className="text-button project-label" title={projectPath} aria-label="Open project folder" disabled={busy}
          onClick={() => perform(() => api("/runs/" + run.id + "/open-folder", {}), "Opening folder…")}><Icon name="folder" />{projectPath.split("/").filter(Boolean).at(-1) || projectPath}</Button>
        {(run.working_folder || tasks.some(t => t.workspace)) && <Button variant="outline" className="project-label" title={run.working_folder || "Milestone checkouts"}
          disabled={busy} onClick={() => perform(() => api("/runs/" + run.id + "/open-folder", {working:true}), "Opening working folder…")}><Icon name="folder" />Open working folder</Button>}
        {run.repo.Target && <code>{branchName(run.repo.Target)}</code>}
        {settings}
      </div>
      {(runTitle(run) !== run.input.prompt || !!run.input.attachments?.length) && <Disclosure key={run.id} className="run-brief" title="Full brief"><Markdown>{run.input.prompt}</Markdown>
        {!!run.input.attachments?.length && <ul aria-label="Brief attachments">{run.input.attachments.map(file => <li key={file.path}>{file.name}</li>)}</ul>}
      </Disclosure>}
    </header>
    <div className="run-body">
    <nav className="run-tabs" aria-label="Run views">
      <Button variant="outline" aria-pressed={view === "work"} onClick={() => setTab("work")}><ListTree/>Milestones</Button>
      <Button variant="outline" aria-pressed={view === "activity"} onClick={() => openActivity(null)}><Activity/>Activity Log</Button>
      <Button variant="outline" aria-pressed={view === "metrics"} onClick={() => setTab("metrics")}><ChartNoAxesCombined/>Metrics</Button>
      <Button variant="outline" aria-pressed={view === "experiment"} onClick={showExperiment}><FlaskConical/>Experiment</Button>
      <Button variant="outline" aria-label="Coordinator chat tab" aria-pressed={view === "chat"} onClick={expandChat}><MessageSquare/>Coordinator chat</Button>
    </nav>
    <Conversation {...{ run, act, busy, api, chatRequest, openMilestone }} expanded={view === "chat"} onExpand={expandChat} onDock={() => setTab(lastView.current)} />
    <div hidden={view !== "work"}>
    {run.detail && !landed && <div className="notice" role="status"><Markdown>{run.detail}</Markdown></div>}
    {landed && (view !== "experiment" || run.selection?.state !== "landed") && <div className="notice landed-receipt" role="status">
      <strong>Implementation landed{run.repo.Target && ` on ${branchName(run.repo.Target)}`}</strong>
      <code aria-label="Landed commit">{run.detail}</code>
    </div>}
    {run.question && run.state === "question" && <Question key={run.question} text={run.question} busy={busy} onAnswer={(text) => act({ kind: "answer", text })} />}
    {run.state === "checks" && <Card asChild><section className="answer decision" aria-labelledby="review-checks-title">
      <h2 id="review-checks-title">Review acceptance checks</h2>
      <p>Approve these checks to start work. Expand a check to see its command.</p>
      <RecipeEditor recipe={run.proposed_recipe} busy={busy} label="Acceptance recipe" submitLabel="Approve checks"
        onApprove={(recipe) => act({ kind: "approve", recipe })} />
    </section></Card>}
    {questions.length > 0 && <div className="notice">
      <span>{questions.length === 1 ? shortSummary(questions[0].text) : `${questions.length} open questions for this run.`}</span>{" "}
      <Button variant="outline" className="text-button" onClick={openInbox}>Open Needs you</Button>
    </div>}
    </div>
    {view === "metrics" && <Metrics key={run.id} run={run} api={api} />}
    {view === "activity" && <ActivityLog initialTurn={activityTurn} {...{run,api}} />}
    {["experiment","work"].includes(view) && <ReleaseList {...{ run, act, busy }} />}
    {experimentOpened && <div hidden={view !== "experiment"}><Experiment active={view === "experiment"} {...{ run, act, perform, api, busy, openRun, explain }} /></div>}
    <div hidden={view !== "work"} className="work-view">
      <Milestones selected={selectedTask} setSelected={setSelectedTask} {...{run,api,act,busy,explain,openActivity}} question={task => <Question text={task.question} busy={busy} onAnswer={text => act({kind:"answer",text,task:task.id})} />}/>
      {(activeCandidates.length > 0 || superseded.length > 0) && <section aria-label="Implementations">
        {activeCandidates.length > 1 && <header className="work-section-heading"><h2>Implementations</h2>
          <Button variant="outline" className="text-button" onClick={showExperiment}>Compare in Experiment</Button></header>}
        {activeCandidates.map(renderCandidate)}
        {superseded.length > 0 && <Disclosure className="superseded-work" title={<>Superseded implementations ({superseded.length})</>}>
          {superseded.map(renderCandidate)}
        </Disclosure>}
      </section>}
    </div>
    </div>
  </section>;
}

const checkAgents = (signal) => api("/setup/status", undefined, signal);
const saveModel = (input) => api("/setup/model", input);

function decisionInputs(run, command) {
  if (["dismiss-issue", "restore-issue"].includes(command.kind)) {
    return JSON.stringify((command.text === "question" ? run.inbox : run.tasks)?.find(item => item.id === command.task));
  }
  if (command.kind === "answer-inbox") return JSON.stringify(run.inbox?.find((q) => q.id === command.task));
  if (["hold-release", "resume-release", "cancel-release"].includes(command.kind)) {
    return JSON.stringify(run.releases?.find((release) => release.id === command.task));
  }
  const candidates = (run.candidates || []).filter((c) =>
    c.id === command.candidate || command.donors?.includes(c.id));
  return JSON.stringify([run.state, run.generation, run.round, run.policy, run.repo.Target, run.landing,
    run.held, run.combination, run.selection, candidates,
    (run.inbox || []).filter((q) => q.state === "open" && q.blocking),
    command.kind === "update-suite" ? run.suite_proposal : null]);
}

function App() {
  const [setup, setSetup] = useState(null);
  const [runs, setRuns] = useState([]);
  const [selection, select] = useState(null);
  const [agentChoice, setAgentChoice] = useState(null);
  const [report, setReport] = useState(null);
  const [checking, setChecking] = useState(true);
  const [checkError, setCheckError] = useState("");
  const agentRequest = useRef(null);
  async function refreshAgents() {
    agentRequest.current?.abort();
    const controller = new AbortController();
    agentRequest.current = controller;
    setChecking(true);
    setCheckError("");
    try {
      const next = await checkAgents(controller.signal);
      if (!controller.signal.aborted) {
        setReport(next);
        setSetup(next.setup);
      }
    } catch (problem) {
      if (!controller.signal.aborted) setCheckError(problem.message);
    } finally {
      if (!controller.signal.aborted) setChecking(false);
    }
  }
  const [draftVersion, setDraftVersion] = useState(0);
  const [startup, setStartup] = useState(0);
  const [connectionError, setConnectionError] = useState(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState("");
  const working = useRef(false);
  const cursor = useRef(0);
  const main = useRef(null);
  const focusPage = useRef(false);
  const [showInboxHistory, setShowInboxHistory] = useState(false);
  const [settingsId, setSettingsId] = useState(null);
  const [dismissedUpdate, setDismissedUpdate] = useState(() => sessionStorage.getItem("local-swarm-dismissed-update") || "");
  const update = setup?.update && setup.update.latest !== dismissedUpdate ? setup.update : null;
  const deleted = useRef(new Set());
  const apply = (run) => {
    if (run.deleted) {
      deleted.current.add(run.id);
      setRuns(items => items.filter(item => item.id !== run.id));
      setSettingsId(id => id === run.id ? null : id);
      select(id => id === run.id ? "" : id);
      return;
    }
    if (deleted.current.has(run.id)) return;
    setRuns((current) => current.some((item) => item.id === run.id) ? current.map((item) => item.id === run.id && (item.revision || 0) <= (run.revision || 0) ? run : item) : [run, ...current]);
  };
  const navigate = (next) => {
    if (next === selection) return;
    focusPage.current = true;
    setShowInboxHistory(false);
    setError("");
    select(next);
  };
  useEffect(() => {
    if (!focusPage.current) return;
    focusPage.current = false;
    const title = [...main.current.querySelectorAll("h1")].find((heading) => !heading.closest("[hidden]"));
    title?.focus();
  }, [selection]);
  const perform = async (fn, message = "Saving…") => {
    if (working.current) return false;
    working.current = true;
    setError("");
    setPending(message);
    try {
      await fn();
      return true;
    } catch (problem) {
      setError(problem.message);
      return false;
    } finally {
      working.current = false;
      setPending("");
    }
  };
  const send = (run, body) => perform(async () => {
    const guarded = ["dismiss-issue", "restore-issue", "answer-inbox", "select", "keep-baseline", "combine", "new-round", "land", "hold", "unhold",
      "hold-release", "resume-release", "cancel-release", "update-suite"].includes(body.kind);
    const path = "/runs/" + run.id;
    const id = body.id || crypto.randomUUID();
    const original = guarded ? decisionInputs(run, body) : "";
    let revision = run.revision;
    for (let attempt = 0; ; attempt++) {
      try {
        apply(await api(path, { ...body, id, ...(guarded ? { expected_revision: revision } : {}) }));
        return;
      } catch (problem) {
        if (!guarded || problem.status !== 409 || attempt >= 2) throw problem;
        const latest = await api(path);
        apply(latest);
        if (original !== decisionInputs(latest, body)) throw problem;
        revision = latest.revision;
      }
    }
  }, body.kind === "land" ? "Landing candidate…" : "Saving…");
  useEffect(() => {
    const controller = new AbortController();
    let timer;
    const sync = async () => {
      let delay = 750;
      try {
        const latest = await api("/events?after=" + cursor.current, undefined, controller.signal);
        if (controller.signal.aborted) return;
        for (const update of latest) { apply(update.run); cursor.current = update.number; }
        if (latest.length >= 100) delay = 0;
        setConnectionError(null);
      } catch (problem) {
        if (!controller.signal.aborted) setConnectionError(problem);
        if (problem.status === 401) return;
      }
      if (!controller.signal.aborted) timer = setTimeout(sync, delay);
    };
    Promise.all([api("/setup", undefined, controller.signal), api("/snapshot", undefined, controller.signal)]).then(([config, snapshot]) => {
      if (controller.signal.aborted) return;
      cursor.current = snapshot.cursor;
      setSetup(config);
      setRuns(snapshot.runs);
      select((current) => current ?? (snapshot.runs.length || config.coordinator ? "" : "setup"));
      refreshAgents();
      sync();
    }).catch((problem) => {
      if (!controller.signal.aborted) setConnectionError(problem);
    });
    return () => { controller.abort(); agentRequest.current?.abort(); clearTimeout(timer); };
  }, [startup]);

  const outstanding = runs.reduce((total, r) => total + (r.inbox || []).filter((q) => q.state === "open" && !q.inbox_dismissed).length + blockedTasks(r).filter(task => !task.inbox_dismissed).length, 0);
  const current = runs.find((run) => run.id === selection);
  const useAgent = (name, config) => {
    setSetup(config);
    setAgentChoice({ name });
    navigate("");
  };
  const onCreate = (run) => {
    apply(run);
    setSetup((config) => ({ ...config, coordinator: run.input.coordinator, workers: run.input.workers }));
    setAgentChoice(null);
    setDraftVersion((value) => value + 1);
    navigate(run.id);
  };
  const updateModel = async (input) => {
    const config = await saveModel(input);
    setSetup(config);
    setReport((current) => current && ({ ...current, setup: config }));
    return config;
  };
  const busy = !!pending;
  const settingsRun = runs.find(run => run.id === settingsId);
  const settingsButton = (run, name) => <Button variant="outline" type="button" className="run-settings-button" aria-label={name} title={name} onClick={() => { setError(""); setSettingsId(run.id); }}><Icon name="settings" /></Button>;
  const navProps = (id) => ({ "aria-current": selection === id ? "page" : undefined, onClick: () => navigate(id) });
  return <div className="app">
    <a className="skip-link" href="#main-content" onClick={(event) => { event.preventDefault(); main.current.focus(); }}>Skip to content</a>
    <WorkbenchHeader project={current?.repo?.Root} connected={!!setup && !connectionError} ready={!!setup} create={()=>navigate("")}/>
    <aside className="sidebar">
      {setup && <>
        <nav className="workspace-nav" aria-label="Workspace">
          {runs.length>0 && <Button variant="ghost" aria-current={current?"page":undefined} onClick={()=>navigate(current?.id || runs[0].id)}><Play/>Runs</Button>}
          <Button variant="outline" {...navProps("setup")}><Icon name="agents" />Agent setup</Button>
          <Button variant="outline" {...navProps("inbox")}><Icon name="inbox" />Needs you{outstanding > 0 && <span className="nav-count"><span className="visually-hidden"> · </span>{outstanding}</span>}</Button>
        </nav>
        {runs.length > 0 && <div className="run-history">
          <div className="nav-heading" aria-hidden="true">Recent runs</div>
          <nav className="run-nav" aria-label="Runs">{runs.map((run) =>
            <div className="run-nav-row" key={run.id}><Button variant="outline" {...navProps(run.id)}><span className="run-name">{runTitle(run)}</span><Status state={run.state} run={run} /></Button>{settingsButton(run, "Settings for " + runTitle(run))}</div>)}</nav>
        </div>}
      </>}
    </aside>
    <main ref={main} tabIndex={-1} id="main-content">
      {update && <div className="notice update-notice" role="status">
        <span>Update available: <strong>Pantheon Local v{update.latest}</strong> (you have v{update.current}). <a href={update.url} target="_blank" rel="noreferrer">View GitHub release ↗</a></span>
        <Button variant="outline" type="button" onClick={() => { sessionStorage.setItem("local-swarm-dismissed-update", update.latest); setDismissedUpdate(update.latest); }} aria-label="Dismiss update notice">×</Button>
      </div>}
      {connectionError && <Alert className="notice error" variant="destructive">
        <span>{connectionError.status === 401 ? connectionError.message : setup ? "Run updates paused. Reconnecting…" : `Couldn’t load Pantheon Local. ${connectionError.message}`}</span>
        {!setup && connectionError.status !== 401 && <Button variant="outline" onClick={() => { setConnectionError(null); setStartup((value) => value + 1); }}>Try again</Button>}
      </Alert>}
      {(error || pending) && <div className="feedback">
        {error && <Alert className="notice error" variant="destructive"><span>{error}</span><Button variant="outline" onClick={() => setError("")} aria-label="Dismiss error">×</Button></Alert>}
        {pending && <Loading top>{pending}</Loading>}
      </div>}
      {setup && <div hidden={selection !== ""}>
        <Brief key={draftVersion} setup={setup} {...{ api, report, checking, checkError }} cwd={sessionStorage.getItem(cwdKey) || setup.cwd || ""} saveModel={updateModel} agentChoice={agentChoice} onSetup={() => navigate("setup")} {...{ onCreate, perform, busy }} />
      </div>}
      {current ? <RunView key={current.id} run={current} settings={settingsButton(current, "Run settings")} openRun={navigate} openInbox={() => {
        navigate("inbox");
        setShowInboxHistory(current.inbox?.some(q => q.state === "open" && q.inbox_dismissed));
      }} {...{ send, perform, busy }} /> :
        selection === "inbox" ? <Inbox runs={runs} initialHistory={showInboxHistory} act={send} open={navigate} busy={busy} /> :
        selection === "setup" ? <AgentSetup {...{ report, checking }} error={checkError} refresh={refreshAgents} saveModel={updateModel} onContinue={useAgent} /> :
        !setup && !connectionError && <Loading top>Connecting to Pantheon Local…</Loading>}
    </main>
    {current && <RunFooter run={current}/>}
    {settingsRun && <RunSettings key={settingsRun.id} run={settingsRun} {...{ send, busy, error }}
      onClose={() => { setSettingsId(null); setError(""); }}
      remove={() => perform(async () => {
        const result = await api("/runs/" + settingsRun.id + "/delete", {expected_revision: settingsRun.revision});
        apply(result);
        if (selection === settingsRun.id) requestAnimationFrame(() => main.current?.querySelector("h1")?.focus());
      }, "Deleting run…")} />}
  </div>;
}

createRoot(document.getElementById("root")).render(<LoadingProvider><App /></LoadingProvider>);
