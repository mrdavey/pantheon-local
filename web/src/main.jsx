import React, { useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";
import { runTitle, blockedTasks } from "./run-summary.js";
import { AgentSetup, profileLabel } from "./setup.jsx";
import { Experiment, Inbox, ReleaseList, title } from "./experiments.jsx";
import { Conversation, RoutingEvidence } from "./conversation.jsx";
import { RecipeEditor, branchName } from "./review.jsx";

const tokenKey = "local-swarm-token";
const token = location.hash.slice(1) || sessionStorage.getItem(tokenKey);
if (location.hash && token) {
  sessionStorage.setItem(tokenKey, token);
  history.replaceState(null, "", location.pathname);
}

async function api(path, body, signal) {
  const response = await fetch("/api" + path, {
    method: body === undefined ? "GET" : "POST",
    headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    const detail = await response.json().catch(() => null);
    const problem = new Error(response.status === 401 ? "Open Pantheon Local from the link printed in Terminal." : detail?.error || response.statusText || `Request failed (${response.status}).`);
    problem.status = response.status;
    throw problem;
  }
  return response.json();
}

const stateLabel = {
  planning: "Planning", question: "Needs an answer", checks: "Review checks", working: "In progress",
  evaluating: "Checking candidates", repairing: "Routing a repair", review: "Ready to review", landed: "Landed",
  budget: "Token target reached", blocked: "Blocked", stopped: "Stopped", pending: "Queued",
  running: "Working", committed: "Committed", interrupted: "Interrupted", passed: "Passed",
  failed: "Failed", conflict: "Merge conflict", inconclusive: "Inconclusive", cancelled: "Canceled",
  superseded: "Superseded", abandoned: "Abandoned", stale: "Stale", queued: "Queued", received: "Finished",
};

const iconPaths = {
  brand: "M3 19h18M5 16V9m5 7V5m5 11V5m4 11V9M3 9l9-6 9 6",
  plus: "M12 5v14M5 12h14",
  agents: "M8 8a4 4 0 1 0 8 0 4 4 0 0 0-8 0ZM4 21v-2a5 5 0 0 1 5-5h6a5 5 0 0 1 5 5v2",
  inbox: "M5 5h14l3 9v5H2v-5l3-9Zm-3 9h6l2 2h4l2-2h6",
  folder: "M3 7h7l2 2h9v10H3V7Z",
};

function Icon({ name }) {
  return <svg className="icon" width="20" height="20" viewBox="0 0 24 24" fill="none"
    stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <path d={iconPaths[name]} />
  </svg>;
}

function Status({ state, run }) {
  if (state === "working" && run && blockedTasks(run).length) return <span className="state blocked">Milestone blocked</span>;
  return <span className={"state " + state}>{stateLabel[state] || state}</span>;
}

function Brief({ setup, agentChoice, onSetup, onCreate, perform, busy }) {
  const [path, setPath] = useState("");
  const [prompt, setPrompt] = useState("");
  const [suggestion, setSuggestion] = useState("");
  const [original, setOriginal] = useState("");
  const [coordinator, setCoordinator] = useState(agentChoice?.name || setup.coordinator || "");
  const [workers, setWorkers] = useState(agentChoice ? [agentChoice.name] : setup.workers || []);
  const [advisor, setAdvisor] = useState("");
  const [tokens, setTokens] = useState("");
  const [routing, setRouting] = useState("manual");
  const [initialFiles, setInitialFiles] = useState("");
  const [target, setTarget] = useState("");
  const [compare, setCompare] = useState(false);
  const [delivery, setDelivery] = useState("review");
  const [destination, setDestination] = useState("");
  const [destinations, setDestinations] = useState(null);
  const [destinationError, setDestinationError] = useState("");
  const [loadingDestinations, setLoadingDestinations] = useState(false);
  useEffect(() => {
    if (agentChoice) {
      setCoordinator(agentChoice.name);
      setWorkers([agentChoice.name]);
    }
  }, [agentChoice]);
  const names = Object.keys(setup.profiles || {}).sort((a, b) => a.localeCompare(b));
  const selectedCoordinator = names.includes(coordinator) ? coordinator : "";
  async function loadDestinations() {
    setLoadingDestinations(true);
    setDestinationError("");
    try {
      setDestinations(await api("/destinations"));
    } catch (problem) {
      setDestinationError(`Couldn’t load destinations. ${problem.message}`);
    } finally {
      setLoadingDestinations(false);
    }
  }
  const selectedWorkers = workers.filter((worker) => names.includes(worker));
  const create = (event) => {
    event.preventDefault();
    if (busy || !selectedWorkers.length || delivery === "deploy" && !destination) return;
    perform(async () => {
      const run = await api("/runs", {
        path, prompt, original_prompt: original, coordinator: selectedCoordinator, workers: selectedWorkers, routing: selectedWorkers.length > 1 ? routing : "manual", token_target: Number(tokens),
        initial_files: initialFiles.split("\n").map((line) => line.trim()).filter(Boolean), target,
        compare, delivery, destination, recipe: { checks: [] },
      });
      onCreate(run);
    }, "Starting run…");
  };
  return <section className="panel brief">
    <h1 tabIndex={-1}>Start a run</h1>
    <form className="form-card" onSubmit={create}>
      <div className="project-picker">
        <label htmlFor="project-folder">Project folder</label>
        <div className="path-controls">
          <input id="project-folder" required value={path} spellCheck={false} autoCapitalize="none" onChange={(event) => setPath(event.target.value)} placeholder="/path/to/project" />
          <button type="button" disabled={busy} onClick={() => perform(async () => {
            const folder = await api("/project-folder", {});
            if (folder) setPath(folder);
          }, "Choosing a folder…")}>Browse…</button>
        </div>
      </div>
      <label>What do you want to build?<textarea required rows={5} value={prompt} onChange={(event) => setPrompt(event.target.value)} /></label>
      <div className="roles">
        <label>Coordinator<select aria-label="Coordinator" required value={selectedCoordinator} onChange={(event) => setCoordinator(event.target.value)}><option value="">Choose a profile</option>{names.map((name) => <option key={name} value={name}>{profileLabel(name, setup.profiles[name])}</option>)}</select></label>
        <fieldset><legend>Workers</legend>{names.map((name) => <label className="checkbox" key={name}><input type="checkbox" checked={selectedWorkers.includes(name)} onChange={(event) => setWorkers(event.target.checked ? [...selectedWorkers, name] : selectedWorkers.filter((worker) => worker !== name))} />{profileLabel(name, setup.profiles[name])}</label>)}</fieldset>
      </div>
      {(!names.length || !selectedWorkers.length) && <p className="muted">
        {names.length ? "Select at least one worker." : "Add an agent profile in setup."}{" "}
        <button type="button" className="text-button" onClick={onSetup}>Agent setup</button>
      </p>}
      {selectedWorkers.length > 1 && <label>Worker routing<select value={routing} onChange={(event) => setRouting(event.target.value)}>
        <option value="manual">Coordinator chooses</option>
        <option value="balanced">Balance benchmark score and cost</option><option value="quality">Highest benchmark score</option>
        <option value="economy">Favor lower cost</option>
      </select></label>}
      <details><summary>Run options</summary>
        <div className="options">
          <label>Token target<input type="number" min="0" step="1" value={tokens} onChange={(event) => setTokens(event.target.value)} placeholder="Optional" /></label>
          <label className="checkbox"><input type="checkbox" checked={compare} onChange={(event) => setCompare(event.target.checked)} />Compare approaches against a baseline</label>
          <label>Delivery<select value={delivery} onChange={(event) => {
            setDelivery(event.target.value);
            if (event.target.value === "deploy" && destinations === null && !loadingDestinations) loadDestinations();
          }}><option value="review">Review before landing</option><option value="auto">Auto-land</option><option value="deploy">Auto-land and deploy</option></select></label>
          {delivery === "deploy" && <div className="destination-field">
            <label>Destination<select value={destination} disabled={loadingDestinations} onChange={(event) => setDestination(event.target.value)}>
              <option value="">Choose a destination</option>{destinations?.map((name) => <option key={name}>{name}</option>)}</select></label>
            {loadingDestinations ? <p role="status">Loading destinations…</p> : destinationError || !destinations?.length ?
              <><p role={destinationError ? "alert" : undefined}>{destinationError || <>Add a destination to <code>destinations.json</code>. Run <code>swarm doctor</code> to find the state directory.</>}</p>
                <button type="button" onClick={loadDestinations}>Reload destinations</button></> :
              <p>The tested commit can land and deploy to this destination automatically.</p>}
          </div>}
        </div>
        <details><summary>Git and advisor settings</summary>
        <div className="options">
          <label>Advisor<select aria-label="Advisor" value={advisor} onChange={(event) => setAdvisor(event.target.value)}><option value="">Use coordinator</option>{names.map((name) => <option key={name} value={name}>{profileLabel(name, setup.profiles[name])}</option>)}</select></label>
          <label>Target branch<input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="Current branch" /></label>
          <label>Initial import files<textarea rows={2} value={initialFiles} onChange={(event) => setInitialFiles(event.target.value)} placeholder="One repo-relative file per line" /></label>
        </div></details>
      </details>
      {suggestion && <div className="answer">
        <label>Suggested brief<textarea rows={4} value={suggestion} onChange={(event) => setSuggestion(event.target.value)} /></label>
        <button type="button" onClick={() => { setOriginal(original || prompt); setPrompt(suggestion); setSuggestion(""); }}>Use suggestion</button>
        <button type="button" onClick={() => setSuggestion("")}>Dismiss</button>
      </div>}
      {original && <details><summary>Original brief</summary><p>{original}</p><button type="button" onClick={() => { setPrompt(original); setOriginal(""); }}>Restore original</button></details>}
      <div className="actions">
        <button className="primary" type="submit" disabled={busy || !path.trim() || !prompt.trim() || !selectedCoordinator || !selectedWorkers.length || delivery === "deploy" && !destination}>Start run</button>
        <button type="button" disabled={busy || !prompt.trim() || !(advisor || selectedCoordinator)} onClick={() => perform(async () => setSuggestion(await api("/advice", { profile: advisor || selectedCoordinator, prompt })), "Improving brief…")}>Improve brief</button>
      </div>
    </form>
  </section>;
}

function Question({ text, onAnswer, busy, submitLabel = "Send answer" }) {
  const [answer, setAnswer] = useState("");
  return <form className="answer" onSubmit={(event) => { event.preventDefault(); onAnswer(answer); }}>
    <label>{text}<textarea required rows={2} value={answer} onChange={(event) => setAnswer(event.target.value)} /></label>
    <button className="primary" type="submit" disabled={busy || !answer.trim()}>{submitLabel}</button>
  </form>;
}

function Candidate({ run, candidate, act, perform, busy }) {
  const [preview, setPreview] = useState("");
  const [diff, setDiff] = useState("");
  const [checks, setChecks] = useState("");
  const evalTurn = (run.turns || []).find((turn) => turn.role === "eval" && turn.task === candidate.id);
  const [changing, setChanging] = useState(false);
  const [target, setTarget] = useState("");
  const needsSelection = !!run.input.recipe.rubric && run.selection?.candidate !== candidate.id;
  const canLand = !candidate.baseline && candidate.eligible !== false && (candidate.policy || 0) === (run.policy || 0) &&
    (candidate.round || 0) === (run.round || 0) && candidate.state === "passed" && run.state === "review";
  const previewCommand = candidate.round ? candidate.preview : run.input.recipe.preview;
  const blocked = (run.inbox || []).some((q) => q.state === "open" && q.blocking &&
    (!q.tasks?.length || q.tasks.some((task) => candidate.tasks?.includes(task))));
  return <article className="row">
    <header><strong>{title(candidate)}</strong><Status state={candidate.state} /></header>
    <p>{candidate.tasks?.filter((task) => task !== title(candidate)).join(" · ")}</p>
    {candidate.detail && <p>{candidate.detail}</p>}
    {!run.repo.Target && canLand && <label>Target branch<input value={target} onChange={(event) => setTarget(event.target.value)} /></label>}
    <div className="actions">
      {candidate.commit && previewCommand?.length > 0 && !preview && <button disabled={busy || candidate.state === "evaluating"} onClick={() => perform(async () => setPreview((await api("/runs/" + run.id + "/preview/" + candidate.id, {})).url), "Starting preview…")}>Start preview</button>}
      {preview && <a href={preview} target="_blank" rel="noreferrer">Open preview ↗</a>}
      {evalTurn?.state === "received" && <button disabled={busy} onClick={() => checks ? setChecks("") : perform(async () => setChecks(await api("/runs/" + run.id + "/turns/" + evalTurn.id)))}>{checks ? "Hide checks" : "View checks"}</button>}
      {candidate.commit && <button disabled={busy} onClick={() => diff ? setDiff("") : perform(async () => setDiff(await api("/runs/" + run.id + "/diff/" + candidate.id)))}>{diff ? "Hide diff" : "View diff"}</button>}
    </div>
    {diff && <pre tabIndex={0} aria-label="Candidate diff">{diff}</pre>}
    {checks && <pre tabIndex={0} aria-label="Acceptance checks">{checks}</pre>}
    {canLand && <>
      {needsSelection && <p>Select this candidate in Experiment after reviewing the criteria.</p>}
      {blocked && <p>Answer the blocking questions before landing this candidate.</p>}
      <div className="actions">
        <button className="primary" disabled={busy || blocked || needsSelection || !!run.combination || (!run.repo.Target && !target.trim())}
          onClick={() => act({ kind: "land", candidate: candidate.id, text: target })}>
          {run.repo.Target ? `Land on ${branchName(run.repo.Target)}` : "Land locally"}
        </button>
        <button aria-expanded={changing} disabled={busy || !!run.combination} onClick={() => setChanging(!changing)}>Request change</button>
      </div>
      {changing && <Question text="What should change?" submitLabel="Request revision" busy={busy}
        onAnswer={async (text) => {
          if (await act({ kind: "change", candidate: candidate.id, text })) setChanging(false);
        }} />}
    </>}
  </article>;
}

const turnLabel = {
  plan: "Planning", observe: "Coordinator review", worker: "Worker", eval: "Evaluation",
  chat: "Conversation", repair: "Repair planning", release: "Release",
};

function RunView({ run, send, perform, busy, openRun, openInbox }) {
  const [log, setLog] = useState(null);
  const [view, setTab] = useState(() => run.selection && (run.input.compare || ["auto", "deploy"].includes(run.input.delivery)) ? "experiment" : "work");
  const [experimentOpened, setExperimentOpened] = useState(view === "experiment");
  const act = (body) => send(run, body);
  const openLog = (id) => perform(async () => setLog({ id, text: await api("/runs/" + run.id + "/turns/" + id) }), "Loading log…");
  const questions = (run.inbox || []).filter((q) => q.state === "open" && (q.source !== "plan" || run.state !== "question"));
  const landed = run.state === "landed" && run.detail;
  const winner = run.candidates?.find((candidate) => candidate.id === run.selection?.candidate);
  const turns = run.turns || [];
  const showExperiment = () => { setExperimentOpened(true); setTab("experiment"); };
  return <section className="panel">
    <header className="run-heading">
      <div className="run-meta">
        <span className="project-label" title={run.repo.Root}><Icon name="folder" />{run.repo.Root.split(/[\\/]/).filter(Boolean).at(-1) || run.repo.Root}</span>
        {run.repo.Target && <code>{branchName(run.repo.Target)}</code>}
        <Status state={run.state} run={run} />
      </div>
      <h1 tabIndex={-1}>{runTitle(run)}</h1>
      {runTitle(run) !== run.input.prompt && <details key={run.id} className="run-brief">
        <summary>Full brief</summary><p>{run.input.prompt}</p>
      </details>}
    </header>
    {run.detail && !landed && <p className="notice" role="status">{run.detail}</p>}
    {landed && (view !== "experiment" || run.selection?.state !== "landed") && <div className="notice landed-receipt" role="status">
      <strong>{winner ? title(winner) : "Implementation"} landed{run.repo.Target && ` on ${branchName(run.repo.Target)}`}</strong>
      <code aria-label="Landed commit">{run.detail}</code>
    </div>}
    {run.question && run.state === "question" && <Question key={run.question} text={run.question} busy={busy} onAnswer={(text) => act({ kind: "answer", text })} />}
    {run.state === "checks" && <section className="answer decision" aria-labelledby="review-checks-title">
      <h2 id="review-checks-title">Review acceptance checks</h2>
      <p>These commands run locally to verify the candidate before it can land.</p>
      <RecipeEditor recipe={run.proposed_recipe} busy={busy} label="Acceptance recipe" submitLabel="Approve checks"
        onApprove={(recipe) => act({ kind: "approve", recipe })} />
    </section>}
    {questions.length > 0 && <div className="notice">
      <span>{questions.length === 1 ? questions[0].text : `${questions.length} open questions for this run.`}</span>{" "}
      <button className="text-button" onClick={openInbox}>Open Needs you</button>
    </div>}
    {run.state !== "landed" && <div className="actions run-actions">
      {["stopped", "blocked", "budget"].includes(run.state) ? <button disabled={busy} onClick={() => act({ kind: "resume", tokens: run.state === "budget" ? 0 : run.input.token_target })}>{run.state === "budget" ? "Continue without target" : "Resume"}</button> : run.state !== "landed" && <button disabled={busy} onClick={() => act({ kind: "stop" })}>Pause run</button>}
      {run.input.delivery && run.input.delivery !== "review" && run.state !== "landed" &&
        <button disabled={busy} onClick={() => act({ kind: "delivery", text: "review" })}>Require review</button>}
    </div>}
    <nav className="run-tabs" aria-label="Run views">
      <button aria-pressed={view === "work"} onClick={() => setTab("work")}>Work</button>
      <button aria-pressed={view === "experiment"} onClick={showExperiment}>Experiment</button>
      <button aria-pressed={view === "conversation"} onClick={() => setTab("conversation")}>Conversation</button>
    </nav>
    <ReleaseList {...{ run, act, busy }} />
    <div hidden={view !== "conversation"}><Conversation {...{ run, act, busy }} /></div>
    {experimentOpened && <div hidden={view !== "experiment"}><Experiment active={view === "experiment"} {...{ run, act, perform, api, busy, openRun }} /></div>}
    <div hidden={view !== "work"}>
      {run.state === "review" && run.input.compare && <div className="notice">
        <button className="text-button" onClick={showExperiment}>Compare candidates in Experiment</button>
      </div>}
      <div className="actions run-controls">
        <label>Coordinator<select disabled={busy || run.state === "landed"} value={run.input.coordinator} onChange={(event) => act({ kind: "coordinator", text: event.target.value })}>{Object.keys(run.profiles).map((name) => <option key={name}>{name}</option>)}</select></label>
        {run.tokens > 0 && <span>{run.tokens.toLocaleString()} recorded tokens</span>}
        {run.unknown_turns > 0 && <span>{run.unknown_turns} turns without usage</span>}
      </div>
      <div className={run.candidates?.length ? "work-grid with-candidates" : "work-grid"}>
        <section className="milestone-list" aria-labelledby="milestone-heading">
          <h2 id="milestone-heading">Milestones</h2>
          {run.benchmark_note && <p>{run.benchmark_note}</p>}
          {!(run.tasks || []).length && <p>Milestones appear after the coordinator proposes a plan.</p>}
          {(run.tasks || []).map((task) => <article className="row" key={task.id}>
            <header><strong>{task.id}</strong><Status state={task.state} /></header>
            <p>{task.goal}</p>
            <div className="milestone-meta">
              <span>{task.worker}{task.variant && " · " + task.variant}</span>
              {task.commit && <code>{task.commit.slice(0, 10)}</code>}
              {task.depends?.length > 0 && <span>After {task.depends.join(", ")}</span>}
            </div>
            <RoutingEvidence route={task.route} />
            {task.finding && <p>{task.finding}</p>}
            {(task.notes || []).map((note, index) => <p key={index}>{note}</p>)}
            {task.question && task.state === "question" && !run.project && <Question text={task.question} busy={busy} onAnswer={(text) => act({ kind: "answer", text, task: task.id })} />}
            {(task.state === "blocked" || task.state === "interrupted") && <button disabled={busy} onClick={() => act({ kind: "resume", task: task.id, tokens: run.input.token_target })}>Retry milestone</button>}
          </article>)}
        </section>
        {run.candidates?.length > 0 && <section className="candidate-list" aria-labelledby="candidate-heading">
          <h2 id="candidate-heading">Candidates</h2>
          {run.candidates.map((candidate) => <Candidate key={candidate.id} {...{ candidate, run, act, perform, busy }} />)}
        </section>}
      </div>
      {turns.length > 0 && <details className="activity-card"><summary>Activity · {turns.length}</summary>
        {[...turns].reverse().map((turn) => <div className="activity-entry" key={turn.id}>
          <div className="activity"><span>{turnLabel[turn.role] || turn.role}{turn.task && " · " + turn.task}</span>
            <Status state={turn.state} />
            <button aria-expanded={log?.id === turn.id} disabled={busy} onClick={() => {
              if (log?.id === turn.id) setLog(null);
              else openLog(turn.id);
            }}>{log?.id === turn.id ? "Hide log" : "View log"}</button>
          </div>
          {log?.id === turn.id && <>
            <pre tabIndex={0} aria-label="Turn log">{log.text || "This turn has no log output yet."}</pre>
            {turn.state === "running" && <button disabled={busy} onClick={() => openLog(turn.id)}>Refresh log</button>}
          </>}
        </div>)}
      </details>}
    </div>
  </section>;
}

const checkAgents = (signal) => api("/setup/status", undefined, signal);
const saveModel = (input) => api("/setup/model", input);

function decisionInputs(run, command) {
  if (command.kind === "answer-inbox") return JSON.stringify(run.inbox?.find((q) => q.id === command.task));
  if (["hold-release", "resume-release"].includes(command.kind)) {
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
  const [draftVersion, setDraftVersion] = useState(0);
  const [startup, setStartup] = useState(0);
  const [connectionError, setConnectionError] = useState(null);
  const [error, setError] = useState("");
  const [pending, setPending] = useState("");
  const working = useRef(false);
  const cursor = useRef(0);
  const main = useRef(null);
  const focusPage = useRef(false);
  const apply = (run) => setRuns((current) => current.some((item) => item.id === run.id) ? current.map((item) => item.id === run.id && (item.revision || 0) <= (run.revision || 0) ? run : item) : [run, ...current]);
  const navigate = (next) => {
    if (next === selection) return;
    focusPage.current = true;
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
    const guarded = ["answer-inbox", "select", "keep-baseline", "combine", "new-round", "land", "hold", "unhold",
      "hold-release", "resume-release", "update-suite"].includes(body.kind);
    const path = "/runs/" + run.id;
    const id = crypto.randomUUID();
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
      try {
        const latest = await api("/events?after=" + cursor.current, undefined, controller.signal);
        if (controller.signal.aborted) return;
        for (const update of latest) { apply(update.run); cursor.current = update.number; }
        setConnectionError(null);
      } catch (problem) {
        if (!controller.signal.aborted) setConnectionError(problem);
        if (problem.status === 401) return;
      }
      if (!controller.signal.aborted) timer = setTimeout(sync, 750);
    };
    Promise.all([api("/setup", undefined, controller.signal), api("/snapshot", undefined, controller.signal)]).then(([config, snapshot]) => {
      if (controller.signal.aborted) return;
      cursor.current = snapshot.cursor;
      setSetup(config);
      setRuns(snapshot.runs);
      select((current) => current ?? (snapshot.runs.length || config.coordinator ? "" : "setup"));
      sync();
    }).catch((problem) => {
      if (!controller.signal.aborted) setConnectionError(problem);
    });
    return () => { controller.abort(); clearTimeout(timer); };
  }, [startup]);

  const outstanding = runs.reduce((total, r) => total + (r.inbox || []).filter((q) => q.state === "open").length + blockedTasks(r).length, 0);
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
    return config;
  };
  const busy = !!pending;
  const navProps = (id) => ({ "aria-current": selection === id ? "page" : undefined, onClick: () => navigate(id) });
  return <div className="app">
    <a className="skip-link" href="#main-content" onClick={(event) => { event.preventDefault(); main.current.focus(); }}>Skip to content</a>
    <aside className="sidebar">
      <div className="brand"><span className="brand-mark"><Icon name="brand" /></span>Pantheon Local</div>
      {setup && <>
        <nav className="workspace-nav" aria-label="Workspace">
          <button {...navProps("")}><Icon name="plus" />New run</button>
          <button {...navProps("setup")}><Icon name="agents" />Agent setup</button>
          <button {...navProps("inbox")}><Icon name="inbox" />Needs you{outstanding > 0 && <span className="nav-count"><span className="visually-hidden"> · </span>{outstanding}</span>}</button>
        </nav>
        {runs.length > 0 && <div className="run-history">
          <div className="nav-heading" aria-hidden="true">Runs</div>
          <nav className="run-nav" aria-label="Runs">{runs.map((run) =>
            <button {...navProps(run.id)} key={run.id}><span className="run-name">{runTitle(run)}</span><Status state={run.state} run={run} /></button>)}</nav>
        </div>}
      </>}
    </aside>
    <main ref={main} tabIndex={-1} id="main-content">
      {connectionError && <div className="notice error" role="alert">
        <span>{connectionError.status === 401 ? connectionError.message : setup ? "Run updates paused. Reconnecting…" : `Couldn’t load Pantheon Local. ${connectionError.message}`}</span>
        {!setup && connectionError.status !== 401 && <button onClick={() => { setConnectionError(null); setStartup((value) => value + 1); }}>Try again</button>}
      </div>}
      {(error || pending) && <div className="feedback">
        {error && <div className="notice error" role="alert"><span>{error}</span><button onClick={() => setError("")} aria-label="Dismiss error">×</button></div>}
        {pending && <p role="status" className="notice">{pending}</p>}
      </div>}
      {setup && <div hidden={selection !== ""}>
        <Brief key={draftVersion} setup={setup} agentChoice={agentChoice} onSetup={() => navigate("setup")} {...{ onCreate, perform, busy }} />
      </div>}
      {current ? <RunView key={current.id} run={current} openRun={navigate} openInbox={() => navigate("inbox")} {...{ send, perform, busy }} /> :
        selection === "inbox" ? <Inbox runs={runs} act={send} open={navigate} busy={busy} /> :
        selection === "setup" ? <AgentSetup check={checkAgents} saveModel={updateModel} onContinue={useAgent} /> :
        !setup && !connectionError && <p role="status">Connecting to Pantheon Local…</p>}
    </main>
  </div>;
}

createRoot(document.getElementById("root")).render(<App />);
