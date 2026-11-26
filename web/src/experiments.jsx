import { Card } from "./components/ui/card";
import { Badge } from "./components/ui/badge";
import { Disclosure } from "./disclosure";
import { Table, TableHeader, TableBody, TableRow, TableCell, TableHead } from "./components/ui/table";
import { Loading } from "./status.jsx";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Textarea } from "./components/ui/textarea";
import { NativeSelect } from "./components/ui/native-select";
import { NativeSelectOption } from "./components/ui/native-select";
import { Label } from "./components/ui/label";
import { Checkbox } from "./components/ui/checkbox";
import { ImplementationDetails, ExperimentHelp, implementationLabel } from "./implementation.jsx";
import { Fields } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import { Dialog } from "./dialog.jsx";
import { runTitle, shortSummary, blockedTasks, canRetryMilestone, retryTokens } from "./run-summary.js";
import React, { useState } from "react";
import { RecipeEditor, branchName } from "./review.jsx";

export const title = implementationLabel;
const number = (value) => new Intl.NumberFormat(undefined, { maximumSignificantDigits: 5 }).format(value);
const when = (value) => new Date(value).toLocaleString();
const rounds = (run) => [...new Set([run.round || 1, ...(run.candidates || []).map((c) => c.round || 1)])].sort((a, b) => a - b);

export function Inbox({ runs, act, open, busy, initialHistory = false }) {
  const [history, setHistory] = useState(initialHistory);
  const items = runs.flatMap(run => [
    ...(run.inbox || []).map(issue => ({ run, issue, type: "question" })),
    ...blockedTasks(run).map(issue => ({ run, issue, type: "milestone" })),
  ]);
  const archived = ({ issue, type }) => issue.inbox_dismissed || (type === "question" && issue.state !== "open");
  const active = items.filter(item => !archived(item));
  const past = items.filter(archived);
  const card = ({ run, issue, type }) => <InboxItem key={run.id + type + issue.id} {...{ run, issue, type, act, open, busy }} />;
  return <section className="panel">
    <h1 tabIndex={-1}>Needs you</h1>
    {active.length ? active.map(card) : <p>No open questions or blockers in your inbox.</p>}
    {past.length > 0 && <><Button variant="outline" aria-expanded={history} onClick={() => setHistory(!history)}>{history ? "Hide history" : "Show history"}</Button>
      {history && <section aria-label="Inbox history">{past.map(card)}</section>}</>}
  </section>;
}

function InboxItem({ run, issue, type, act, open, busy }) {
  const [answer, setAnswer] = useState("");
  const resolved = type === "question" && issue.state !== "open";
  const dismiss = <Button variant="outline" type="button" disabled={busy} onClick={() => act(run, {
    kind: issue.inbox_dismissed ? "restore-issue" : "dismiss-issue", task: issue.id, text: type,
  })}>{issue.inbox_dismissed ? "Restore to inbox" : "Dismiss"}</Button>;
  return <article className="row">
    <Button variant="outline" className="text-button" onClick={() => open(run.id)}>{runTitle(run)}</Button>
    {type === "question" ? <>
      <h2>{issue.text}</h2>
      {resolved ? <Markdown>{issue.answer || issue.state}</Markdown> : <form onSubmit={async event => {
        event.preventDefault();
        if (await act(run, {kind: "answer-inbox", task: issue.id, text: answer})) setAnswer("");
      }}>
        {issue.choices?.length > 0 && <div className="actions">{issue.choices.map((choice, i) =>
          <Button variant="outline" key={i} type="button" onClick={() => setAnswer(choice)}>{choice}</Button>)}</div>}
        <Label>Answer<Textarea required value={answer} onChange={event => setAnswer(event.target.value)} /></Label>
        {issue.tasks?.length > 0 && <p>Waiting work · {issue.tasks.join(", ")}</p>}
        {issue.blocking && <p className="muted">This run needs an answer to continue.</p>}
        <div className="actions"><Button variant="default" className="primary" disabled={busy || !answer.trim()}>Send answer</Button>{dismiss}</div>
      </form>}
    </> : <>
      <h2>{issue.title || issue.id}</h2><p>Blocked · {issue.worker}</p>
      {issue.finding && <Markdown>{issue.finding}</Markdown>}
      <div className="actions">{canRetryMilestone(run, issue) && <Button variant="outline" disabled={busy} onClick={() => act(run, { kind: "resume", task: issue.id, tokens: retryTokens(run) })}>Retry milestone</Button>}{dismiss}</div>
    </>}
  </article>;
}

function ScorePlot({ values, metric, xMetric, yLimit, xLimit }) {
  const [point, setPoint] = useState(null);
  const isTradeoff = Boolean(xMetric);
  const published = values.filter((point) => point.y !== undefined && point.x !== undefined);
  if (!published.length) return <p>No matching measurements.</p>;
  const range = (values) => [Math.min(...values), Math.max(...values)];
  const [xmin, xmax] = range([...published.map((p) => p.x), ...(xLimit === undefined ? [] : [xLimit])]);
  const [ymin, ymax] = range([...published.flatMap((p) => [p.min ?? p.y, p.max ?? p.y]), ...(yLimit === undefined ? [] : [yLimit])]);
  const x = (value) => 15 + 75 * (xmax === xmin ? .5 : (value - xmin) / (xmax - xmin));
  const y = (value) => 12 + 76 * (ymax === ymin ? .5 : (value - ymin) / (ymax - ymin));
  const active = published[point];
  return <figure className="plot">
    <figcaption>{metric} {isTradeoff ? "by " + xMetric : "across rounds"}</figcaption>
    <div className="plot-area">
      <span className="chart-high" style={{ bottom: y(ymax) + "%" }}>{number(ymax)}</span>
      {ymin !== ymax && <span className="chart-low" style={{ bottom: y(ymin) + "%" }}>{number(ymin)}</span>}
      {yLimit !== undefined && <span className="limit horizontal" style={{ bottom: y(yLimit) + "%" }} title={`Limit ${number(yLimit)}`} />}
      {xLimit !== undefined && <span className="limit vertical" style={{ left: x(xLimit) + "%" }} title={`Limit ${number(xLimit)}`} />}
      {published.map((entry, j) => <React.Fragment key={entry.id}>
        {entry.min !== undefined && entry.max !== entry.min &&
          <span className="spread" style={{ left: x(entry.x) + "%", bottom: y(entry.min) + "%", height: (y(entry.max) - y(entry.min)) + "%" }} />}
        <Button variant="outline" type="button" className={`point ${entry.kind || ""}`}
          style={{ left: x(entry.x) + "%", bottom: y(entry.y) + "%" }}
          aria-label={`${entry.name}, ${number(entry.y)} ${metric}`}
          aria-describedby={isTradeoff ? `point-${entry.id}-x` : undefined}
          onPointerEnter={() => setPoint(j)} onFocus={() => setPoint(j)} onClick={() => setPoint(j)} />
        {isTradeoff && <span className="visually-hidden" id={`point-${entry.id}-x`}>{number(entry.x)} {xMetric}</span>}
      </React.Fragment>)}
      <span className="axis-min" style={{ left: x(xmin) + "%" }}>{number(xmin)}</span>
      {xmin !== xmax && <span className="axis-max" style={{ left: x(xmax) + "%" }}>{number(xmax)}</span>}
    </div>
    <p className="plot-reading" aria-live="polite">{active && <>{active.name} · {number(active.y)}{xMetric && ` · ${number(active.x)} ${xMetric}`}{active.n > 0 && ` · ${active.n} samples`}</>}</p>
    {(xLimit !== undefined || yLimit !== undefined) && <p className="plot-limit">Limits {xLimit !== undefined && `${xMetric} ${number(xLimit)} `}{yLimit !== undefined && `${metric} ${number(yLimit)}`}</p>}
  </figure>;
}

function Charts({ run, candidates, rule, policy, includeSuperseded }) {
  const metrics = rule?.metrics || [];
  const [chosen, setChosen] = useState("");
  const [second, setSecond] = useState("");
  const y = metrics.find((m) => m.name === chosen) || metrics.find((m) => m.name === rule?.objective);
  const x = metrics.find((m) => m.name === second && m !== y) || metrics.find((m) => m !== y);
  if (!y) return null;
  const label = (m) => `${m.name} (${m.unit})`;
  const history = (run.candidates || []).filter((c) => (c.policy || 0) === policy && (includeSuperseded || c.state !== "superseded"));
  const showHistory = !x || new Set(history.filter((c) => c.score?.[y.name]).map((c) => c.round)).size > 1;
  return <Card asChild><section className="charts-card" aria-labelledby="metrics-heading">
    <h3 id="metrics-heading">Measurements</h3>
    <div className="actions chart-controls">
      <Label>Metric<NativeSelect value={y.name} onChange={(event) => setChosen(event.target.value)}>{metrics.map((m) => <NativeSelectOption key={m.name} value={m.name}>{label(m)}</NativeSelectOption>)}</NativeSelect></Label>
      {x && <Label>Compare with<NativeSelect value={x.name} onChange={(event) => setSecond(event.target.value)}>{metrics.filter((m) => m !== y).map((m) => <NativeSelectOption key={m.name} value={m.name}>{label(m)}</NativeSelectOption>)}</NativeSelect></Label>}
    </div>
    <div className="charts">
      {showHistory && <ScorePlot metric={label(y)} values={history.map((c) => ({ id: c.id, name: title(c), x: c.round, y: c.score?.[y.name]?.value, kind: c.baseline ? "baseline" : c.id === run.selection?.candidate ? "winner" : "", ...c.score?.[y.name] }))} />}
      {x && <ScorePlot xMetric={label(x)} metric={label(y)} yLimit={rule.limits?.[y.name]} xLimit={rule.limits?.[x.name]}
        values={candidates.map((c) => ({ id: c.id, name: title(c), x: c.score?.[x.name]?.value, y: c.score?.[y.name]?.value, kind: c.baseline ? "baseline" : c.id === run.selection?.candidate ? "winner" : "", ...c.score?.[y.name] }))} />}
    </div>
  </section></Card>;
}

function Cases({ run, candidates, api }) {
  const [reports, setReports] = useState(null);
  const [filter, setFilter] = useState("All cases");
  const withEvidence = candidates.filter((c) => c.evidence);
  const [failed, setFailed] = useState(0);
  const [loading, setLoading] = useState(false);
  const load = async () => {
    if (loading) return;
    setLoading(true);
    try {
      const results = await Promise.allSettled(withEvidence.map(async (c) =>
        [c, await api("/runs/" + run.id + "/evidence/" + c.id)]));
      const available = results.filter((result) => result.status === "fulfilled");
      setReports(available.map((result) => result.value));
      setFailed(results.length - available.length);
    } finally {
      setLoading(false);
    }
  };
  if (!withEvidence.length) return null;
  if (!reports) return <Button variant="outline" disabled={loading} onClick={load}>{loading ? "Loading case results…" : "Compare case results"}</Button>;
  const keys = [...new Set(reports.flatMap(([, report]) => (report.samples || []).map((s) => s.case)))];
  const rows = keys.map((name) => {
    const cells = reports.map(([, report]) => (report.samples || []).filter((s) => s.case === name));
    const rate = (samples) => samples.length && samples.every((s) => s.state !== "error")
      ? samples.filter((s) => s.state === "pass").length / samples.length : null;
    const baseIndex = reports.findIndex(([c]) => c.baseline);
    const baseline = cells[baseIndex] ? rate(cells[baseIndex]) : null;
    const regressed = baseline !== null && cells.some((cell) => rate(cell) !== null && rate(cell) < baseline);
    const improved = baseline !== null && cells.some((cell) => rate(cell) !== null && rate(cell) > baseline);
    return { name, cells, regressed, improved };
  });
  const filtered = rows.filter(({ regressed, improved }) => filter === "All cases" || (filter === "Regressions" ? regressed : improved));
  return <Card asChild><section className="form-card">
    <h3>Case results</h3>
    {failed > 0 && <div className="notice">
      <p role="status">Couldn’t load case results for {failed} {failed === 1 ? "implementation" : "implementations"}.</p>
      <Button variant="outline" disabled={loading} onClick={load}>Reload case results</Button>
    </div>}
    {keys.length > 0 && <Label>Show<NativeSelect value={filter} onChange={(event) => setFilter(event.target.value)}>
      <NativeSelectOption>All cases</NativeSelectOption><NativeSelectOption>Improvements</NativeSelectOption><NativeSelectOption>Regressions</NativeSelectOption>
    </NativeSelect></Label>}
    {!filtered.length && <p>{filter === "All cases" ? "No case samples available." : `No ${filter.toLowerCase()} in the loaded results.`}</p>}
    {filtered.length > 0 && <div className="table-scroll" tabIndex={0} role="region" aria-label="Case comparison">
      <Table><TableHeader><TableRow><TableHead scope="col">Case</TableHead>{reports.map(([c]) => <TableHead scope="col" key={c.id}>{title(c)}</TableHead>)}</TableRow></TableHeader>
        <TableBody>{filtered.map(({ name, cells }) => <TableRow key={name}><TableHead scope="row">{name}</TableHead>{cells.map((samples, i) =>
            <TableCell key={reports[i][0].id}>{samples.length ? <Disclosure title={<>{samples.filter((s) => s.state === "pass").length}/{samples.length} passed</>}>
              {samples.map((s) => <div key={s.number}><strong>Sample {s.number} · {s.state}</strong>
                {s.observed && <p>{s.observed}</p>}{s.error && <p>{s.error}</p>}
                {s.expected != null && <div><strong>Expected</strong><Fields value={s.expected} /></div>}
                {Object.entries(s.values || {}).map(([metric, value]) => <p key={metric}>{metric} · {number(value)}</p>)}</div>)}
            </Disclosure> : "Missing"}</TableCell>)}</TableRow>)}</TableBody></Table>
    </div>}
  </section></Card>;
}

export function Experiment({ active, run, act, perform, api, busy, openRun, explain }) {
  const [base, setBase] = useState("");
  const [donors, setDonors] = useState([]);
  const [goal, setGoal] = useState("");
  const [mode, setMode] = useState("");
  const [feedback, setFeedback] = useState("");
  const [rollout, setRollout] = useState(false);
  const [target, setTarget] = useState("");
  const [detail, setDetail] = useState("");
  const [savedCases, setSavedCases] = useState(null);
  const [loadingCases, setLoadingCases] = useState(false);
  const [casesError, setCasesError] = useState("");
  const [ab, setAB] = useState({});
  const [history, setHistory] = useState(null);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [historyError, setHistoryError] = useState("");
  const [inspected, setInspected] = useState("");
  const [expandedGroup, setExpandedGroup] = useState("");
  const [round, setRound] = useState(0);
  const [policy, setPolicy] = useState(null);
  const visibleRound = round || run.round || 1;
  const all = run.candidates || [];
  const period = all.filter((c) => (c.round || 1) === visibleRound);
  const policies = [...new Set(period.map((c) => c.policy || 0))].sort((a, b) => a - b);
  const latestPolicy = policies.includes(policy) && policy !== null ? policy : policies.at(-1) ?? (visibleRound === (run.round || 1) ? run.policy || 0 : 0);
  const candidates = period.filter((c) => (c.policy || 0) === latestPolicy);
  const votes = (run.preferences || []).filter((p) => typeof p.agreement === "boolean" && p.round === visibleRound);
  const implemented = all.filter((c) => c.commit && !c.baseline);
  const current = candidates.filter((c) => c.commit && !c.baseline);
  const previewable = current.filter((c) => (c.round ? c.preview : run.input.recipe.preview)?.length);
  const isCurrent = visibleRound === (run.round || 1) && latestPolicy === (run.policy || 0);
  const matchesView = (selection) => (selection?.round || 1) === visibleRound && (selection?.policy || 0) === latestPolicy;
  const selected = isCurrent ? run.selection : run.past_selections?.findLast(matchesView);
  const winner = selected?.state === "baseline" ? candidates.findLast((c) => c.baseline) :
    candidates.find((c) => c.id === (selected?.candidate || selected?.recommended));
  const canDecide = isCurrent && run.state === "review" && !run.combination;
  const landingBlocked = (run.inbox || []).some((q) => q.state === "open" && q.blocking &&
    (!q.tasks?.length || q.tasks.some((task) => winner?.tasks?.includes(task))));
  const rule = run.policies?.find((entry) => entry.revision === latestPolicy)?.rule ||
    (latestPolicy === run.policy ? run.input.recipe.rule : null);
  const loadCases = async (before = "") => {
    if (loadingCases) return;
    setLoadingCases(true);
    setCasesError("");
    try {
      const batch = await api("/runs/" + run.id + "/cases" + (before ? "?before=" + before : ""));
      setSavedCases((current) => before && current ? [...current, ...batch] : batch);
    } catch (problem) {
      setCasesError(problem.message);
    } finally {
      setLoadingCases(false);
    }
  };
  const loadHistory = async (before = "") => {
    if (loadingHistory) return;
    setLoadingHistory(true);
    setHistoryError("");
    try {
      const batch = await api("/runs/" + run.id + "/history" + (before ? "?before=" + before : ""));
      setHistory((current) => before && current ? [...current, ...batch] : batch);
    } catch (problem) {
      setHistoryError(problem.message);
    } finally {
      setLoadingHistory(false);
    }
  };
  const evalRunning = (run.turns || []).some((t) => t.state === "running" && (t.role === "eval" || (t.role === "worker" && run.state === "evaluating")));
  const canUpdateSuite = !run.landing && !run.combination && !evalRunning && (
    ["working", "evaluating", "review", "blocked", "repairing", "budget"].includes(run.state) ||
    (run.state === "stopped" && !["landed", "planning", "question", "checks"].includes(run.stopped_from))
  );
  const headline = winner && title(winner);
  const inspectedCandidate = all.find(c => c.id === inspected);
  const group = visibleRound + ":" + latestPolicy;
  const superseded = candidates.filter(c => c.state === "superseded");
  const visibleCandidates = expandedGroup === group ? candidates : candidates.filter(c => c.state !== "superseded");
  const reason = shortSummary(selected?.reason);
  return <section className="experiment">
    <h2 className="visually-hidden">Experiment</h2>
    {isCurrent && run.round_goal && <div className="round-goal"><Markdown>{run.round_goal}</Markdown></div>}
    {selected && <Card asChild><div className="outcome">
      <div className="outcome-copy">
        <Badge asChild variant="outline"><strong className={`state ${selected.state}`}>{selected.state === "landed" ? "Landed" : selected.state === "selected" ? "Selected" :
          selected.state === "recommended" ? "Recommended" : selected.state === "baseline" ? "Keeping the baseline" : "Awaiting a decision"}</strong></Badge>
        {winner && <h3>{headline}</h3>}
        {reason && <p>{reason}</p>}
        <Button variant="outline" className="text-button" onClick={() => setDetail("decision")}>View decision details</Button>
        {detail === "decision" && <ImplementationDetails candidate={winner} decision={selected} {...{run, rule, explain}} onClose={() => setDetail("")} />}
      </div>
      {(winner?.commit || selected.landed_at) && <div className="outcome-meta">
        {selected.landed_at && <span><strong>{branchName(run.repo.Target)}</strong> · {when(selected.landed_at)}</span>}
        {winner?.commit && <code aria-label={active && isCurrent && run.state === "landed" && selected.state === "landed" ? "Landed commit" : undefined}>
          {isCurrent && run.state === "landed" && selected.state === "landed" ? run.detail : winner.commit}
        </code>}
        {isCurrent && run.started && selected.landed_at && <span>{run.interventions || 0} developer actions · {number((Date.parse(selected.landed_at) - Date.parse(run.started)) / 1000)} seconds to land</span>}
      </div>}
    </div></Card>}
    {run.observer_error && <div role="status"><p>{run.observer_error}</p><Button variant="outline" disabled={busy} onClick={() => act({ kind: "retry-coordinator" })}>Retry coordinator</Button></div>}
    {run.combination && <div role="status"><p>Waiting for the coordinator to plan the combination.</p>
      <Button variant="outline" disabled={busy} onClick={() => act({ kind: "cancel-combination" })}>Cancel request</Button></div>}
    {rounds(run).length > 1 && <Label>Round<NativeSelect value={visibleRound} onChange={(event) => { setRound(Number(event.target.value)); setPolicy(null); setMode(""); }}>{rounds(run).map((r) => <NativeSelectOption key={r}>{r}</NativeSelectOption>)}</NativeSelect></Label>}
    {candidates.length > 0 ? <>
      {policies.length > 1 ? <Label>Suite revision<NativeSelect value={latestPolicy} onChange={(event) => { setPolicy(Number(event.target.value)); setMode(""); }}>
        {policies.map((entry) => <NativeSelectOption key={entry}>{entry}</NativeSelectOption>)}</NativeSelect></Label> :
        run.policies?.length > 1 && <p>Suite revision {latestPolicy}</p>}
      <div className={rule?.metrics?.length ? "experiment-results with-charts" : "experiment-results"}>
        <Card asChild><section className="comparison-card" aria-labelledby="comparison-heading">
          <h3 id="comparison-heading">Implementations</h3>
          <div className="table-scroll" role="region" aria-label="Implementation comparison" tabIndex={0}>
            <Table><TableHeader><TableRow><TableHead scope="col">Implementation</TableHead><TableHead scope="col">Checks</TableHead>{rule?.metrics?.map((m) => <TableHead scope="col" key={m.name}>{m.name} ({m.unit})</TableHead>)}<TableHead scope="col">Decision</TableHead></TableRow></TableHeader>
              <TableBody>{visibleCandidates.map((c) => <TableRow key={c.id} className={c.id === selected?.candidate ? "open-row selected-row" : "open-row"} onClick={event => {
                if (!event.target.closest("button,a")) {
                  event.currentTarget.querySelector("button").focus({preventScroll:true});
                  setInspected(c.id);
                }
              }}>
                <TableHead scope="row"><Button variant="outline" className="text-button implementation-title" aria-label={"View details for " + title(c)} onClick={() => setInspected(c.id)}>{title(c)}</Button></TableHead>
                <TableCell><Badge asChild variant="outline"><span className={`state ${c.state}`}>{c.state}</span></Badge></TableCell>
                {rule?.metrics?.map((m) => <TableCell key={m.name}>{c.score?.[m.name] ? number(c.score[m.name].value) : "—"}</TableCell>)}
                <TableCell>
                  {c.id === selected?.candidate ? (selected.state === "landed" ? "Landed" : "Selected") :
                    c.id === selected?.recommended ? "Recommended" :
                    c.baseline ? "Baseline" : c.state === "passed" && c.eligible === false ? "Doesn’t qualify" : ""}
                  {canDecide && !c.baseline && c.state === "passed" && c.eligible !== false && c.id !== selected?.candidate &&
                    <Button variant="outline" disabled={busy} onClick={() => act({ kind: "select", candidate: c.id })}>Select</Button>}
                </TableCell>
              </TableRow>)}</TableBody></Table>
          </div>
          {superseded.length > 0 && <Button variant="outline" className="text-button superseded-toggle" aria-expanded={expandedGroup === group}
            onClick={() => setExpandedGroup(expandedGroup === group ? "" : group)}>
            {expandedGroup === group ? "Hide" : "Show"} superseded implementations ({superseded.length})
          </Button>}
          <ExperimentHelp />
        </section></Card>
        <Charts {...{ run, rule }} candidates={visibleCandidates} policy={latestPolicy} includeSuperseded={expandedGroup === group} />
      </div>
      {inspectedCandidate && <ImplementationDetails candidate={inspectedCandidate} {...{run, rule, explain}} onClose={() => setInspected("")} />}
    </> : <p>Measurements appear as candidates finish.</p>}
    <div className="actions">
      {canDecide && <Button variant="outline" disabled={busy} onClick={() => act({ kind: "keep-baseline" })}>Keep current version</Button>}
      {canDecide && winner?.eligible && !winner.baseline && ["selected", "recommended"].includes(selected?.state) && (!run.input.delivery || run.input.delivery === "review" || run.held || !run.repo.Target) &&
        <>{!run.repo.Target && <Label>Target branch<Input value={target} onChange={(event) => setTarget(event.target.value)} /></Label>}
          <Button variant="default" className="primary" disabled={busy || landingBlocked || (!run.repo.Target && !target.trim())}
            onClick={() => act({ kind: "land", candidate: winner.id, text: target })}>
            {run.repo.Target ? `Land selected on ${branchName(run.repo.Target)}` : "Land selected implementation"}
          </Button></>}
      {run.state !== "landed" && run.input.delivery && run.input.delivery !== "review" &&
        <Button variant="outline" disabled={busy || !!run.combination} onClick={() => act({ kind: run.held ? "unhold" : "hold" })}>{run.held ? "Resume auto-land" : "Hold auto-land"}</Button>}
      {isCurrent && implemented.length > 1 && (canDecide || run.state === "landed") &&
        <Button variant="outline" className={run.state === "landed" ? "primary" : undefined} aria-expanded={mode === "combine"} disabled={busy || !!run.combination} onClick={() => { setBase((!winner?.baseline && winner?.id) || current[0]?.id || ""); setDonors([]); setMode(mode === "combine" ? "" : "combine"); }}>
          {run.state === "landed" ? "Improve this version" : "Combine aspects"}
        </Button>}
      {canDecide && run.input.recipe.rubric && previewable.length > 1 &&
        <Button variant="outline" onClick={() => { setBase(previewable[0].id); setDonors([previewable[1].id]); setMode("blind"); }}>Blind review</Button>}
      {isCurrent && (canDecide || run.state === "landed") &&
        <Button variant="outline" aria-expanded={mode === "next"} disabled={busy} onClick={() => { setMode(mode === "next" ? "" : "next"); setGoal(""); }}>Start another round</Button>}
    </div>
    {canDecide && run.input.recipe.rubric && <p>Review criteria · {run.input.recipe.rubric}</p>}
    {mode === "next" && <form className="answer" onSubmit={(event) => {
      event.preventDefault(); act({ kind: "new-round", text: goal }).then((ok) => { if (ok) setMode(""); });
    }}>
      <Label>What should the swarm try next?<Textarea required value={goal} onChange={(event) => setGoal(event.target.value)} /></Label>
      <Button variant="default" className="primary" disabled={busy || !goal.trim()}>Start round</Button>
    </form>}
    {mode === "combine" || mode === "blind" ? <form className="answer" onSubmit={(event) => {
      event.preventDefault();
      act({ kind: mode === "combine" ? "combine" : "blind-review", candidate: base, donors, text: goal }).then((ok) => { if (ok) setMode(""); });
    }}>
      <Label>Start from<NativeSelect required value={base} disabled={run.state === "landed"} onChange={(event) => {
        setBase(event.target.value); setDonors(donors.filter((d) => d !== event.target.value));
      }}><NativeSelectOption value="">Choose implementation</NativeSelectOption>{(mode === "blind" ? previewable : implemented).map((c) => <NativeSelectOption key={c.id} value={c.id}>{title(c)}</NativeSelectOption>)}</NativeSelect></Label>
      <fieldset><legend>{mode === "combine" ? "Take aspects from" : "Compare against"}</legend>
        {(mode === "blind" ? previewable : implemented).filter((c) => c.id !== base).map((c) => <Label key={c.id} className="checkbox">
          <Input type={mode === "combine" ? "checkbox" : "radio"} name="donors" checked={donors.includes(c.id)} onChange={(event) =>
            setDonors(mode === "blind" ? [c.id] : event.target.checked ? [...donors, c.id] : donors.filter((id) => id !== c.id))} />{title(c)}</Label>)}
      </fieldset>
      {mode === "combine" && <Label>What should carry over?<Textarea required value={goal} onChange={(event) => setGoal(event.target.value)} /></Label>}
      <Button variant="default" className="primary" disabled={busy || !base || !donors.length || mode === "combine" && !goal.trim()}>{mode === "combine" ? "Build combination" : "Start review"}</Button>
      <Button variant="outline" type="button" onClick={() => setMode("")}>Cancel</Button>
    </form> : null}
    {candidates.length > 0 &&
      <div><Cases key={candidates.map((c) => c.id + c.evidence).join(":")} {...{ run, candidates, api, perform }} /></div>}
    {votes.length > 0 && <p>{votes.filter((p) => p.agreement).length} of {votes.length} blind preferences agreed with the measured recommendation.</p>}
    {(run.preferences || []).map((p) => <article className="row" key={p.id}>
      <h3>Blind review</h3><p>{p.rubric}</p>
      {p.choice ? <p>{p.choice === "tie" || p.choice === "skip" ? p.choice : title(all.find((c) => c.id === p[p.choice])) + " preferred"}</p> :
        <><div className="actions">{["a", "b"].map((side) => {
          const key = p.id + side;
          return ab[key] ? <a key={side} href={ab[key]} target="_blank" rel="noreferrer">Open {side.toUpperCase()}</a> :
            <Button variant="outline" disabled={busy} key={side} onClick={() => perform(async () => {
              const { url } = await api(`/runs/${run.id}/blind/${p.id}/${side}`, {});
              setAB((current) => ({ ...current, [key]: url }));
            })}>Load {side.toUpperCase()}</Button>;
        })}</div>
        <div className="actions">{[["left", "Prefer A"], ["right", "Prefer B"], ["tie", "Tie"], ["skip", "Skip"]].map(([value, label]) =>
          <Button variant="outline" key={value} disabled={busy} onClick={() => act({ kind: "vote", task: p.id, text: value })}>{label}</Button>)}</div></>}
    </article>)}
    <div className="actions experiment-tools">
      <Button variant="outline" onClick={() => setDetail("suite")}>Eval suite{run.suite_proposal ? " · proposal ready" : ""}</Button>
      <Button variant="outline" onClick={() => { setDetail("cases"); if (!savedCases) loadCases(); }}>Project eval cases</Button>
      <Button variant="outline" onClick={() => { setDetail("history"); if (!history) loadHistory(); }}>Project run history</Button>
    </div>
    {detail === "suite" && <Dialog title="Eval suite" onClose={() => setDetail("")}>
      <RecipeEditor key={run.policy} recipe={run.suite_proposal || run.input.recipe} label="Suite" submitLabel="Approve suite revision"
        busy={busy || !canUpdateSuite}
        onApprove={async (recipe) => { const ok = await act({ kind: "update-suite", recipe }); if (ok) setDetail(""); return ok; }} />
    </Dialog>}
    {detail === "cases" && <Dialog title="Project eval cases" onClose={() => setDetail("")}>
      {casesError && <p role="alert" className="notice error">{casesError}</p>}
      {!savedCases && (loadingCases ? <Loading>Loading cases…</Loading> : <Button variant="outline" onClick={() => loadCases()}>Reload cases</Button>)}
      {savedCases && !savedCases.length && <p>No saved cases for this project.</p>}
      {savedCases?.map((entry) => <article className="row" key={entry.cursor}><strong>{entry.scenario}</strong>
        <p>Expected · {entry.expected}</p><Markdown>{entry.why}</Markdown><small>{entry.active ? "Active in this suite" : entry.state} · Revision {entry.revision} · {entry.author}</small></article>)}
      {savedCases?.length >= 100 && savedCases.length % 100 === 0 && <Button variant="outline" disabled={loadingCases} onClick={() => loadCases(savedCases.at(-1).cursor)}>Load older revisions</Button>}
    </Dialog>}
    {detail === "history" && <Dialog title="Project run history" onClose={() => setDetail("")}>
      {historyError && <p role="alert" className="notice error">{historyError}</p>}
      {!history && (loadingHistory ? <Loading>Loading history…</Loading> : <Button variant="outline" onClick={() => loadHistory()}>Reload history</Button>)}
      <ul>{history?.map((entry) => <li key={entry.id}><Button variant="outline" disabled={entry.id === run.id}
        onClick={() => openRun(entry.id)}>{runTitle(entry)} · {entry.state}</Button></li>)}</ul>
      {history?.length >= 20 && history.length % 20 === 0 && <Button variant="outline" disabled={loadingHistory} onClick={() => loadHistory(history.at(-1).cursor)}>Load older runs</Button>}
    </Dialog>}
    <Card asChild><form className="form-card" onSubmit={(event) => {
      event.preventDefault(); act({ kind: "feedback", text: feedback, candidate: winner?.id, task: rollout ? "rollout" : "" })
        .then((ok) => { if (ok) setFeedback(""); });
    }}>
      <Label>Feedback for the coordinator<Textarea value={feedback} onChange={(event) => setFeedback(event.target.value)} /></Label>
      <Label className="checkbox"><Checkbox checked={rollout} onCheckedChange={setRollout} />From a live rollout</Label>
      <Button variant="outline" disabled={!feedback.trim() || busy}>Send feedback</Button>
    </form></Card>
  </section>;
}

const releaseLabels = {
  queued: "Queued", building: "Building artifact", built: "Artifact verified", "reading-baseline": "Checking destination",
  staged: "Ready to deploy", deploying: "Deploying", verify: "Awaiting live check", verifying: "Checking live artifact",
  "needs-rollback": "Rollback needed", "rolling-back": "Rolling back", "verify-rollback": "Awaiting rollback check",
  "checking-rollback": "Checking rollback", uncertain: "Needs live check", reconcile: "Awaiting live check",
  reconciling: "Checking live state", deployed: "Live and healthy", failed: "Failed", "rolled-back": "Rolled back",
  superseded: "Superseded", cancelled: "Canceled",
};

export function ReleaseList({ run, act, busy }) {
  if (!run.releases?.length) return null;
  return <section>
    <h2>Releases</h2>
    {run.releases.map((release) => {
      const running = (run.turns || []).some((t) => t.state === "running" && t.role === "release" && t.task === release.id);
      return <article key={release.id} className="row">
        <header><strong>{release.destination}</strong><Badge asChild variant="outline"><span className={`state ${release.state === "deployed" ? "passed" : release.state === "failed" ? "failed" : release.state === "uncertain" || release.held ? "question" : ""}`}>
          {release.held ? "On hold" : releaseLabels[release.state] || release.state}
        </span></Badge></header>
        <p><code>{release.commit}</code></p>
        {release.digest && <p>Artifact <code>{release.digest}</code></p>}
        {release.observed && <p>Live artifact <code>{release.observed}</code></p>}
        {release.detail && <p>{release.detail}</p>}
        {release.at && <p>{when(release.at)}</p>}
        <div className="actions">
          {["queued", "building", "built", "reading-baseline", "staged"].includes(release.state) &&
            <Button variant="outline" disabled={busy} onClick={() => act({ kind: release.held ? "resume-release" : "hold-release", task: release.id })}>{release.held ? "Resume release" : "Hold release"}</Button>}
          {!running && ["queued", "built", "staged"].includes(release.state) &&
            <Button variant="outline" disabled={busy} onClick={() => act({ kind: "cancel-release", task: release.id })}>Cancel release</Button>}
          {release.state === "uncertain" && <Button variant="outline" disabled={busy} onClick={() => act({ kind: "reconcile-release", task: release.id })}>Check live state</Button>}
        </div>
      </article>;
    })}
  </section>;
}
