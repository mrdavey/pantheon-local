import { Card } from "./components/ui/card";
import { Alert } from "./components/ui/alert";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Textarea } from "./components/ui/textarea";
import { Label } from "./components/ui/label";
import {
  Status, Loading, LoadingProvider, conversationLoadingMessage, preparingMilestones,
  runLoadingMessage, runStatus, statusLabel, waitingForCoordinator,
} from "./status.jsx";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "./components/ui/tooltip";
import { WorkbenchHeader, RunFooter } from "./workbench-shell";
import { PackageCheck, ListTree, Activity, ChartNoAxesCombined, FlaskConical, MessageSquare, Users, Play, PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { changeRoute, useRoute, destination } from "./navigation";
import { RunsIndex } from "./runs-index";
import { ActivityLog } from "./activity-log";
import { RunAgents } from "./run-agents";
import { Milestones } from "./milestones";
import { Metrics } from "./metrics";
import { ImplementationFiles } from "./implementation-files";
import { implementationLabel } from "./implementation.jsx";
import { Dialog } from "./dialog.jsx";
import React, { useEffect, useRef, useState } from "react";
import { RunSettings } from "./run-settings.jsx";
import { createRoot } from "react-dom/client";
import "./style.css";
import "./work.css";
import { DiffExplorer, TurnOutput } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import { runTitle, runProjectPath, runInitials, shortSummary, blockedTasks } from "./run-summary.js";
import { AgentSetup } from "./setup.jsx";
import { Brief } from "./brief.jsx";
import { Experiment, Inbox, ReleaseList } from "./experiments.jsx";
import { Conversation } from "./conversation.jsx";
import { branchName } from "./review.jsx";

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
  history.replaceState(null, "", location.pathname + location.search);
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

function Candidate({ route, run, candidate, act, busy, name, landed, explain, openMilestone, verifyImplementation }) {
  const evalTurn = (run.turns || []).findLast((turn) => turn.role === "eval" && (turn.task === candidate.id || turn.id === candidate.evidence));
  const panel = route.tab === "implementations" && route.candidate === candidate.id ? route.detail : "";
  const show = detail => changeRoute({candidate:detail ? candidate.id : null, detail, file:null});
  const [target, setTarget] = useState("");
  const needsSelection = !!run.input.recipe.rubric && run.selection?.candidate !== candidate.id;
  const canLand = !candidate.baseline && candidate.eligible !== false && (candidate.policy || 0) === (run.policy || 0) &&
    (candidate.round || 0) === (run.round || 0) && candidate.state === "passed" && run.state === "review" && !run.suite_proposal;
  const tasks = (candidate.tasks || []).map(id => run.tasks?.find(t => t.id === id)).filter(Boolean);
  const terminalTasks = tasks.filter(t => !tasks.some(next => next.depends?.includes(t.id)));
  const finalTask = tasks.find(t => t.commit === candidate.commit) ||
    terminalTasks.sort((a,b) => (a.completed_at || 0) - (b.completed_at || 0)).at(-1);
  const blocked = (run.inbox || []).some((q) => q.state === "open" && q.blocking &&
    (!q.tasks?.length || q.tasks.some((task) => candidate.tasks?.includes(task))));
  const evidence = panel === "diff" && candidate.commit ? {kind:"diff", path:"/runs/" + run.id + "/diff/" + candidate.id} :
    panel === "checks" && evalTurn ? {kind:"checks", path:"/runs/" + run.id + "/turns/" + evalTurn.id} : null;
  return <Card asChild><article className="row work-result">
    <header>{name ? <h3>{name}</h3> : <h2>Implementation</h2>}<div className="implementation-badges">
      <Status state={candidate.state} />{landed && <Status state="landed" />}
    </div></header>
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
        <Button variant="outline" disabled={busy || !!run.combination} onClick={() => show("revision")}>Request change</Button>
      </div>
      {panel === "revision" && <Dialog title="Request a change" onClose={() => show(null)}>
        <Question text="What should change?" submitLabel="Request revision" busy={busy} onAnswer={async (text) => {
          if (await act({ kind: "change", candidate: candidate.id, text })) show(null);
        }} /></Dialog>}
    </>}
    {landed && <div className="actions">
      <Button variant="default" className="primary" onClick={() => changeRoute({page:"new", from:run.id, implementation:candidate.id}, {reset:true})}>Start new run</Button>
      <Button variant="outline" disabled={busy} onClick={() => verifyImplementation(run, candidate)}>Find and fix bugs</Button>
    </div>}
    <div className="actions result-tools">
      {candidate.commit && <Button variant="outline" disabled={busy} onClick={() => show("diff")}>View diff</Button>}
      {evalTurn && <Button variant="outline" disabled={busy} onClick={() => show("checks")}>View checks</Button>}
      {finalTask && <Button variant="outline" onClick={() => openMilestone(finalTask.id)}>View final milestone</Button>}
      <Button onClick={() => show("files")}>View details</Button>
    </div>
    {panel === "files" && <ImplementationFiles {...{api,run,candidate,explain}} path={route.file} onClose={() => show(null)} />}
    {evidence && <Dialog title={evidence.kind === "diff" ? "File changes" : "Acceptance checks"} className="evidence-dialog" onClose={() => show(null)}>
      <Evidence {...evidence} name="Acceptance checks" refreshable={evidence.kind === "checks" && evalTurn?.state === "running"} />
    </Dialog>}
  </article></Card>;
}

function successful(run) {
  const byId = new Map((run.candidates || []).map(candidate => [candidate.id, candidate]));
  const ordered = new Set([...(run.landed_implementation_ids || []), ...(run.implementation_ids || [])]);
  return Array.from(ordered, id => byId.get(id)).filter(Boolean);
}
function defaultTab(run) {
  if (run.implementation_ids?.length) return "implementations";
  return run.selection && (run.input.compare || ["auto", "deploy"].includes(run.input.delivery)) ? "experiment" : "milestones";
}

function RunView({ run, send, perform, busy, openRun, openInbox, settings, route, verifyImplementation }) {
  const tabs = {milestones:"work",agents:"agents",chat:"chat",activity:"activity",metrics:"metrics",experiment:"experiment"};
  const implementations = successful(run);
  if (implementations.length) tabs.implementations = "implementations";
  const fallback = tabs[defaultTab(run)];
  useEffect(() => {
    if (!Object.hasOwn(tabs, route.tab)) changeRoute({tab: defaultTab(run)}, {replace:true});
  }, [run.id, route.tab, implementations.length]);
  const view = Object.hasOwn(tabs, route.tab) ? tabs[route.tab] : fallback;
  const setTab = (next, extra = {}, options) => changeRoute({...destination(run.id), tab: next === "work" ? "milestones" : next, ...extra}, {reset:true, ...options});
  const [experimentOpened, setExperimentOpened] = useState(view === "experiment");
  const lastView = useRef(null);
  const selectedTask = view === "work" ? route.milestone : null;
  const setSelectedTask = (milestone, extra = {}) => changeRoute({tab:"milestones", milestone, turn:null, logPage:null, ...extra});
  const openMilestone = milestone => setTab("work", {milestone});
  const openActivity = (role = "") => setTab("activity", {role});
  const expandChat = () => { if (view !== "chat") lastView.current = route; setTab("chat"); };
  const openProposalReply = reply => {
    lastView.current = {...route, add:null, proposalRequest:null};
    setTab("chat", {reply}, {replace:true});
  };
  const [chatRequest, setChatRequest] = useState(null);
  const explain = (kind, id) => {
    setChatRequest({kind, id, key: crypto.randomUUID()});
  };
  const act = (body) => send(run, body);
  const questions = (run.inbox || []).filter((q) => q.state === "open" && (q.source !== "plan" || run.state !== "question"));
  const landed = run.state === "landed" && run.detail;
  const showExperiment = () => { setExperimentOpened(true); setTab("experiment"); };

  const projectPath = runProjectPath(run);
  return <section className="panel run-workspace">
    <header className="run-heading">
      <div className="run-crumb"><a href="?page=runs" onClick={event => { if (!(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)) {event.preventDefault(); openRun("runs");} }}>Runs</a> <span>/</span> {run.id.slice(0,8)}</div>
      <div className="run-title"><h1 tabIndex={-1}>{runTitle(run)}</h1><Status state={run.state} run={run}/></div>
      <div className="run-meta">
        <Button variant="outline" className="text-button project-label" title={projectPath} aria-label="Open project folder" disabled={busy}
          onClick={() => perform(() => api("/runs/" + run.id + "/open-folder", {}), "Opening folder…")}><Icon name="folder" />{projectPath.split("/").filter(Boolean).at(-1) || projectPath}</Button>
        {settings}
      </div>
    </header>
    <div className="run-body">
    <nav className="run-tabs" aria-label="Run views">
      {implementations.length > 0 && <Button variant="outline" aria-pressed={view === "implementations"} onClick={() => setTab("implementations")}><PackageCheck/>Implementations</Button>}
      <Button variant="outline" aria-pressed={view === "work"} onClick={() => setTab("work")}><ListTree/>Milestones</Button>
      <Button variant="outline" aria-pressed={view === "agents"} onClick={() => setTab("agents")}><Users/>Agents</Button>
      <Button variant="outline" aria-label="Coordinator chat tab" aria-pressed={view === "chat"} onClick={expandChat}><MessageSquare/>Coordinator chat</Button>
      <Button variant="outline" aria-pressed={view === "activity"} onClick={() => openActivity()}><Activity/>Activity Log</Button>
      <Button variant="outline" aria-pressed={view === "metrics"} onClick={() => setTab("metrics")}><ChartNoAxesCombined/>Metrics</Button>
      <Button variant="outline" aria-pressed={view === "experiment"} onClick={showExperiment}><FlaskConical/>Experiment</Button>
    </nav>
    <Conversation {...{ run, act, busy, api, chatRequest, openMilestone }} onCoordinatorActivity={() => openActivity("coordinator")} expanded={view === "chat"} replyId={route.reply} inspectAgents={view === "agents"} onExpand={expandChat} onDock={() => lastView.current ? changeRoute(lastView.current, {reset:true}) : setTab("work")} />
    <div hidden={view !== "work"}>
    {run.detail && !landed && <div className="notice" role="status"><Markdown>{run.detail}</Markdown></div>}
    {run.question && run.state === "question" && <Question key={run.question} text={run.question} busy={busy} onAnswer={(text) => act({ kind: "answer", text })} />}
    {questions.length > 0 && <div className="notice">
      <span>{questions.length === 1 ? shortSummary(questions[0].text) : `${questions.length} open questions for this run.`}</span>{" "}
      <Button variant="outline" className="text-button" onClick={openInbox}>Open Needs you</Button>
    </div>}
    </div>
    {view === "implementations" && <section aria-label="Successful implementations">
      {implementations.map(candidate => <Candidate key={candidate.id} name={implementationLabel(candidate)} landed={run.landed_implementation_ids?.includes(candidate.id)}
        {...{route,candidate,run,act,busy,explain,openMilestone,verifyImplementation}} />)}
    </section>}
    {view === "metrics" && <Metrics key={run.id} run={run} api={api} />}
    {view === "agents" && <RunAgents key={run.id} {...{run, route, api, openMilestone}} />}
    {view === "activity" && <ActivityLog route={route} {...{run,api,explain}} />}
    {["experiment","work"].includes(view) && <ReleaseList {...{ run, act, busy }} />}
    {(experimentOpened || view === "experiment") && <div hidden={view !== "experiment"}><Experiment active={view === "experiment"} {...{ run, act, perform, api, busy, openRun, explain }} /></div>}
    <div hidden={view !== "work"} className="work-view">
      <Milestones route={route} selected={selectedTask} setSelected={setSelectedTask} {...{run,api,act,busy,explain,openProposalReply}} question={task => <Question text={task.question} busy={busy} onAnswer={text => act({kind:"answer",text,task:task.id})} />}/>
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
  const route = useRoute();
  const selection = route.page === "run" ? route.run : route.page === "new" ? "" : route.page;
  const [collapsed, setCollapsed] = useState(() => localStorage.getItem("pantheon-sidebar") === "icons");
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
  const settingsId = route.settings;
  const [dismissedUpdate, setDismissedUpdate] = useState(() => sessionStorage.getItem("local-swarm-dismissed-update") || "");
  const update = setup?.update && setup.update.latest !== dismissedUpdate ? setup.update : null;
  const deleted = useRef(new Set());
  const apply = (run) => {
    if (run.deleted) {
      deleted.current.add(run.id);
      setRuns(items => items.filter(item => item.id !== run.id));
      const currentRoute = new URLSearchParams(location.search);
      if (currentRoute.get("run") === run.id) changeRoute({page:"runs"}, {reset:true, replace:true});
      else if (currentRoute.get("settings") === run.id) changeRoute({settings:null, brief:null}, {replace:true});
      return;
    }
    if (deleted.current.has(run.id)) return;
    setRuns((current) => current.some((item) => item.id === run.id) ? current.map((item) => item.id === run.id && (item.revision || 0) <= (run.revision || 0) ? run : item) : [run, ...current]);
  };
  const navigate = (next) => {
    if (next === selection) return;
    focusPage.current = true;
    setError("");
    const target = runs.find(run => run.id === next);
    const tab = target ? defaultTab(target) : undefined;
    changeRoute({...destination(next), tab}, {reset:true});
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
  }, ({
    land: "Landing candidate…",
    "propose-milestones": preparingMilestones,
    chat: waitingForCoordinator,
  })[body.kind] || "Saving…");
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
      if (!new URLSearchParams(location.search).has("page")) {
        changeRoute({page: snapshot.runs.length ? "runs" : config.coordinator ? "new" : "setup"}, {reset:true, replace:true});
      }
      refreshAgents();
      sync();
    }).catch((problem) => {
      if (!controller.signal.aborted) setConnectionError(problem);
    });
    return () => { controller.abort(); agentRequest.current?.abort(); clearTimeout(timer); };
  }, [startup]);

  useEffect(() => {
    if (setup && !["run","setup","inbox","new","runs"].includes(route.page)) changeRoute({page:runs.length ? "runs" : "new"}, {reset:true, replace:true});
    if (setup && !runs.length && selection === "runs") changeRoute({page:"new"}, {reset:true, replace:true});
  }, [setup, runs.length, selection, route.page]);
  const outstanding = runs.reduce((total, r) => total + (r.inbox || []).filter((q) => q.state === "open" && !q.inbox_dismissed).length + blockedTasks(r).filter(task => !task.inbox_dismissed).length, 0);
  const current = runs.find((run) => run.id === selection);
  const loadingRun = connectionError?.status !== 401 &&
    ((current && runLoadingMessage(current)) || runs.map(conversationLoadingMessage).find(Boolean));
  const followup = route.page === "new" && route.from ? runs.find(run =>
    run.id === route.from && run.landed_implementation_ids?.includes(route.implementation)) : null;
  const verifyImplementation = (run, candidate) => perform(async () => {
    const next = await api("/runs/" + run.id + "/implementations/" + candidate.id + "/verify", {});
    apply(next);
    navigate(next.id);
  }, "Starting correctness checks…");
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
  const settingsButton = (run) => <Button variant="outline" type="button" className="run-settings-button" aria-label="Run settings" title="Run settings" onClick={() => { setError(""); changeRoute({settings:run.id, brief:null}); }}><Icon name="settings" /></Button>;
  const navProps = (id) => ({ "aria-current": selection === id ? "page" : undefined, onClick: () => navigate(id) });
  return <div className="app" data-collapsed={collapsed}>
    {!setup && !connectionError && <Loading top>Connecting to Pantheon Local…</Loading>}
    {setup && connectionError && connectionError.status !== 401 && <Loading top>Reconnecting to Pantheon Local…</Loading>}
    {setup && checking && <Loading top>Checking local agents…</Loading>}
    {loadingRun && <Loading top>{loadingRun}</Loading>}
    {pending && <Loading top>{pending}</Loading>}
    <a className="skip-link" href="#main-content" onClick={(event) => { event.preventDefault(); main.current.focus(); }}>Skip to content</a>
    <WorkbenchHeader project={current?.repo?.Root} connected={!!setup && !connectionError} ready={!!setup} />
    <aside className="sidebar">
      {setup && <>
        <Button className="rail-toggle" variant="ghost" size="icon" aria-expanded={!collapsed} aria-controls="workspace-navigation" aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"} title={collapsed ? "Expand sidebar" : "Collapse sidebar"} onClick={() => {
          localStorage.setItem("pantheon-sidebar", collapsed ? "expanded" : "icons"); setCollapsed(!collapsed);
        }}>{collapsed ? <PanelLeftOpen/> : <PanelLeftClose/>}</Button>
        <div className="sidebar-content">
          <nav id="workspace-navigation" className="workspace-nav" aria-label="Workspace">
            <Button variant="ghost" aria-label="Runs" title="Runs" aria-current={["runs","new","run"].includes(route.page) ? "page" : undefined} onClick={()=>navigate(runs.length ? "runs" : "")}><Play/><span>Runs</span></Button>
            <Button variant="outline" aria-label="Agent setup" title="Agent setup" {...navProps("setup")}><Icon name="agents" /><span>Agent setup</span></Button>
            <Button variant="outline" title="Needs you" {...navProps("inbox")}><Icon name="inbox" /><span>Needs you</span>{outstanding > 0 && <span className="nav-count"><span className="visually-hidden"> · </span>{outstanding}</span>}</Button>
          </nav>
          {runs.length > 0 && <div className="run-history">
            <div className="nav-heading" aria-hidden="true">Recent runs</div>
            <nav className="run-nav" aria-label="Runs"><TooltipProvider delayDuration={200}>{runs.slice(0,8).map((run) => {
              const title = runTitle(run, { truncate: !collapsed });
              const shortcut = <Button variant="outline" title={collapsed ? undefined : title} {...navProps(run.id)}>
                <span className="recent-initials" aria-hidden="true">{runInitials(title)}</span>
                <span className="run-name">{title}</span>
                <Status run={run} compact={collapsed} />
              </Button>;
              return <div className="run-nav-row" key={run.id}>
                {collapsed ? <Tooltip>
                  <TooltipTrigger asChild>{shortcut}</TooltipTrigger>
                  <TooltipContent side="right" className="recent-run-tooltip">
                    <strong>{title}</strong><span>{statusLabel(runStatus(run))}</span>
                  </TooltipContent>
                </Tooltip> : shortcut}
              </div>;
            })}</TooltipProvider></nav>
          </div>}
        </div>
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
      {error && <div className="feedback">
        <Alert className="notice error" variant="destructive"><span>{error}</span><Button variant="outline" onClick={() => setError("")} aria-label="Dismiss error">×</Button></Alert>
      </div>}
      {setup && selection === "" && route.from && !followup && <section><h1>Implementation unavailable</h1>
        <Button onClick={() => navigate("runs")}>Back to runs</Button></section>}
      {setup && !(selection === "" && route.from && !followup) && <div hidden={selection !== ""}>
        <Brief key={followup ? followup.id + "/" + route.implementation : draftVersion} followup={followup} setup={setup} {...{ api, report, checking, checkError }} cwd={sessionStorage.getItem(cwdKey) || setup.cwd || ""} agentChoice={agentChoice} onSetup={() => navigate("setup")} {...{ onCreate, perform, busy }} />
      </div>}
      {selection === "runs" && setup && <RunsIndex {...{runs,navigate,route}}/>}
      {setup && route.page === "run" && !current && <section><h1>Run unavailable</h1><Button onClick={()=>navigate(runs.length ? "runs" : "")}>Back to runs</Button></section>}
      {current ? <RunView route={route} key={current.id} run={current} verifyImplementation={verifyImplementation} settings={settingsButton(current)} openRun={navigate} openInbox={() => {
        navigate("inbox");
        if (current.inbox?.some(q => q.state === "open" && q.inbox_dismissed)) changeRoute({history:"open"}, {replace:true});
      }} {...{ send, perform, busy }} /> :
        selection === "inbox" ? <Inbox runs={runs} act={send} open={navigate} busy={busy} /> :
        selection === "setup" ? <AgentSetup api={api} {...{ report, checking }} error={checkError} refresh={refreshAgents} saveModel={updateModel} onContinue={useAgent} /> :
        null}
    </main>
    {current && <RunFooter run={current}/>}
    {settingsRun && <RunSettings key={settingsRun.id} run={settingsRun} {...{ send, busy, error, api }} briefOpen={route.brief === "open"} setBriefOpen={open => changeRoute({brief:open ? "open" : null})}
      openWorking={() => perform(() => api("/runs/" + settingsRun.id + "/open-folder", {working:true}), "Opening working folder…")}
      onClose={() => { changeRoute({settings:null, brief:null}); setError(""); }}
      remove={() => perform(async () => {
        const result = await api("/runs/" + settingsRun.id + "/delete", {expected_revision: settingsRun.revision});
        apply(result);
        if (selection === settingsRun.id) requestAnimationFrame(() => main.current?.querySelector("h1")?.focus());
      }, "Deleting run…")} />}
  </div>;
}

createRoot(document.getElementById("root")).render(<LoadingProvider><App /></LoadingProvider>);
