import { Fields } from "./evidence.jsx";
import { Markdown } from "./markdown.jsx";
import { Dialog } from "./dialog.jsx";
import { runTitle, shortSummary, blockedTasks } from "./run-summary.js";
import React, { useState } from "react";
import { RecipeEditor, branchName } from "./review.jsx";

export const title = (candidate) => candidate?.name || candidate?.tasks?.join(" · ") || candidate?.id || "Implementation";
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
    {past.length > 0 && <><button aria-expanded={history} onClick={() => setHistory(!history)}>{history ? "Hide history" : "Show history"}</button>
      {history && <section aria-label="Inbox history">{past.map(card)}</section>}</>}
  </section>;
}

function InboxItem({ run, issue, type, act, open, busy }) {
  const [answer, setAnswer] = useState("");
  const resolved = type === "question" && issue.state !== "open";
  const dismiss = <button type="button" disabled={busy} onClick={() => act(run, {
    kind: issue.inbox_dismissed ? "restore-issue" : "dismiss-issue", task: issue.id, text: type,
  })}>{issue.inbox_dismissed ? "Restore to inbox" : "Dismiss"}</button>;
  return <article className="row">
    <button className="text-button" onClick={() => open(run.id)}>{runTitle(run)}</button>
    {type === "question" ? <>
      <h2>{issue.text}</h2>
      {resolved ? <Markdown>{issue.answer || issue.state}</Markdown> : <form onSubmit={async event => {
        event.preventDefault();
        if (await act(run, {kind: "answer-inbox", task: issue.id, text: answer})) setAnswer("");
      }}>
        {issue.choices?.length > 0 && <div className="actions">{issue.choices.map((choice, i) =>
          <button key={i} type="button" onClick={() => setAnswer(choice)}>{choice}</button>)}</div>}
        <label>Answer<textarea required value={answer} onChange={event => setAnswer(event.target.value)} /></label>
        {issue.tasks?.length > 0 && <p>Waiting work · {issue.tasks.join(", ")}</p>}
        {issue.blocking && <p className="muted">This run needs an answer to continue.</p>}
        <div className="actions"><button className="primary" disabled={busy || !answer.trim()}>Send answer</button>{dismiss}</div>
      </form>}
    </> : <>
      <h2>{issue.title || issue.id}</h2><p>Blocked · {issue.worker}</p>
      {issue.finding && <Markdown>{issue.finding}</Markdown>}
      <div className="actions"><button disabled={busy} onClick={() => act(run, { kind: "resume", task: issue.id, tokens: run.input.token_target })}>Retry milestone</button>{dismiss}</div>
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
        <button type="button" className={`point ${entry.kind || ""}`}
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

function Charts({ run, candidates, rule, policy }) {
  const metrics = rule?.metrics || [];
  const [chosen, setChosen] = useState("");
  const [second, setSecond] = useState("");
  const y = metrics.find((m) => m.name === chosen) || metrics.find((m) => m.name === rule?.objective);
  const x = metrics.find((m) => m.name === second && m !== y) || metrics.find((m) => m !== y);
  if (!y) return null;
  const label = (m) => `${m.name} (${m.unit})`;
  const history = (run.candidates || []).filter((c) => (c.policy || 0) === policy);
  const showHistory = !x || new Set(history.filter((c) => c.score?.[y.name]).map((c) => c.round)).size > 1;
  return <section className="charts-card" aria-labelledby="metrics-heading">
    <h3 id="metrics-heading">Measurements</h3>
    <div className="actions chart-controls">
      <label>Metric<select value={y.name} onChange={(event) => setChosen(event.target.value)}>{metrics.map((m) => <option key={m.name} value={m.name}>{label(m)}</option>)}</select></label>
      {x && <label>Compare with<select value={x.name} onChange={(event) => setSecond(event.target.value)}>{metrics.filter((m) => m !== y).map((m) => <option key={m.name} value={m.name}>{label(m)}</option>)}</select></label>}
    </div>
    <div className="charts">
      {showHistory && <ScorePlot metric={label(y)} values={history.map((c) => ({ id: c.id, name: title(c), x: c.round, y: c.score?.[y.name]?.value, kind: c.baseline ? "baseline" : c.id === run.selection?.candidate ? "winner" : "", ...c.score?.[y.name] }))} />}
      {x && <ScorePlot xMetric={label(x)} metric={label(y)} yLimit={rule.limits?.[y.name]} xLimit={rule.limits?.[x.name]}
        values={candidates.map((c) => ({ id: c.id, name: title(c), x: c.score?.[x.name]?.value, y: c.score?.[y.name]?.value, kind: c.baseline ? "baseline" : c.id === run.selection?.candidate ? "winner" : "", ...c.score?.[y.name] }))} />}
    </div>
  </section>;
}

function Cases({ run, candidates, api, perform }) {
  const [reports, setReports] = useState(null);
  const [filter, setFilter] = useState("All cases");
  const withEvidence = candidates.filter((c) => c.evidence);
  const [failed, setFailed] = useState(0);
  const load = () => perform(async () => {
    const results = await Promise.allSettled(withEvidence.map(async (c) =>
      [c, await api("/runs/" + run.id + "/evidence/" + c.id)]));
    const available = results.filter((result) => result.status === "fulfilled");
    setReports(available.map((result) => result.value));
    setFailed(results.length - available.length);
  }, "Loading case results…");
  if (!withEvidence.length) return null;
  if (!reports) return <button onClick={load}>Compare case results</button>;
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
  return <section className="form-card">
    <h3>Case results</h3>
    {failed > 0 && <div className="notice">
      <p role="status">Couldn’t load case results for {failed} {failed === 1 ? "implementation" : "implementations"}.</p>
      <button onClick={load}>Reload case results</button>
    </div>}
    {keys.length > 0 && <label>Show<select value={filter} onChange={(event) => setFilter(event.target.value)}>
      <option>All cases</option><option>Improvements</option><option>Regressions</option>
    </select></label>}
    {!filtered.length && <p>{filter === "All cases" ? "No case samples available." : `No ${filter.toLowerCase()} in the loaded results.`}</p>}
    {filtered.length > 0 && <div className="table-scroll" tabIndex={0} role="region" aria-label="Case comparison">
      <table><thead><tr><th scope="col">Case</th>{reports.map(([c]) => <th scope="col" key={c.id}>{title(c)}</th>)}</tr></thead>
        <tbody>{filtered.map(({ name, cells }) => <tr key={name}><th scope="row">{name}</th>{cells.map((samples, i) =>
            <td key={reports[i][0].id}>{samples.length ? <details><summary>{samples.filter((s) => s.state === "pass").length}/{samples.length} passed</summary>
              {samples.map((s) => <div key={s.number}><strong>Sample {s.number} · {s.state}</strong>
                {s.observed && <p>{s.observed}</p>}{s.error && <p>{s.error}</p>}
                {s.expected != null && <div><strong>Expected</strong><Fields value={s.expected} /></div>}
                {Object.entries(s.values || {}).map(([metric, value]) => <p key={metric}>{metric} · {number(value)}</p>)}</div>)}
            </details> : "Missing"}</td>)}</tr>)}</tbody></table>
    </div>}
  </section>;
}

export function Experiment({ active, run, act, perform, api, busy, openRun }) {
  const [base, setBase] = useState("");
  const [donors, setDonors] = useState([]);
  const [goal, setGoal] = useState("");
  const [mode, setMode] = useState("");
  const [feedback, setFeedback] = useState("");
  const [rollout, setRollout] = useState(false);
  const [target, setTarget] = useState("");
  const [detail, setDetail] = useState("");
  const [savedCases, setSavedCases] = useState(null);
  const [ab, setAB] = useState({});
  const [history, setHistory] = useState(null);
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
  const loadCases = (before = "") => perform(async () => {
    const batch = await api("/runs/" + run.id + "/cases" + (before ? "?before=" + before : ""));
    setSavedCases(before ? [...savedCases, ...batch] : batch);
  });
  const loadHistory = (before = "") => perform(async () => {
    const batch = await api("/runs/" + run.id + "/history" + (before ? "?before=" + before : ""));
    setHistory(before ? [...history, ...batch] : batch);
  });
  const outcomeTitle = winner && title(winner);
  const combined = winner?.tasks?.length > 1 && outcomeTitle === winner.tasks.join(" · ");
  const headline = combined ? "Combined implementation" : shortSummary(outcomeTitle, 80);
  const reason = shortSummary(selected?.reason);
  return <section className="experiment">
    <h2 className="visually-hidden">Experiment</h2>
    {isCurrent && run.round_goal && <div className="round-goal"><Markdown>{run.round_goal}</Markdown></div>}
    {selected && <div className="outcome">
      <div className="outcome-copy">
        <strong className={`state ${selected.state}`}>{selected.state === "landed" ? "Landed" : selected.state === "selected" ? "Selected" :
          selected.state === "recommended" ? "Recommended" : selected.state === "baseline" ? "Keeping the baseline" : "Awaiting a decision"}</strong>
        {winner && <h3>{headline}</h3>}
        {reason && <p>{reason}</p>}
        {(headline !== (outcomeTitle || "") || reason !== (selected.reason || "").trim()) &&
          <button className="text-button" onClick={() => setDetail("decision")}>View decision details</button>}
        {detail === "decision" && <Dialog title="Decision details" onClose={() => setDetail("")}>
          {winner && <h3>{outcomeTitle}</h3>}<Markdown>{selected.reason}</Markdown>
        </Dialog>}
      </div>
      {(winner?.commit || selected.landed_at) && <div className="outcome-meta">
        {selected.landed_at && <span><strong>{branchName(run.repo.Target)}</strong> · {when(selected.landed_at)}</span>}
        {winner?.commit && <code aria-label={active && isCurrent && run.state === "landed" && selected.state === "landed" ? "Landed commit" : undefined}>
          {isCurrent && run.state === "landed" && selected.state === "landed" ? run.detail : winner.commit}
        </code>}
        {isCurrent && run.started && selected.landed_at && <span>{run.interventions || 0} developer actions · {number((Date.parse(selected.landed_at) - Date.parse(run.started)) / 1000)} seconds to land</span>}
      </div>}
    </div>}
    {run.observer_error && <div role="status"><p>{run.observer_error}</p><button disabled={busy} onClick={() => act({ kind: "retry-coordinator" })}>Retry coordinator</button></div>}
    {run.combination && <div role="status"><p>Waiting for the coordinator to plan the combination.</p>
      <button disabled={busy} onClick={() => act({ kind: "cancel-combination" })}>Cancel request</button></div>}
    {rounds(run).length > 1 && <label>Round<select value={visibleRound} onChange={(event) => { setRound(Number(event.target.value)); setPolicy(null); setMode(""); }}>{rounds(run).map((r) => <option key={r}>{r}</option>)}</select></label>}
    {candidates.length > 0 ? <>
      {policies.length > 1 ? <label>Suite revision<select value={latestPolicy} onChange={(event) => { setPolicy(Number(event.target.value)); setMode(""); }}>
        {policies.map((entry) => <option key={entry}>{entry}</option>)}</select></label> :
        run.policies?.length > 1 && <p>Suite revision {latestPolicy}</p>}
      <div className={rule?.metrics?.length ? "experiment-results with-charts" : "experiment-results"}>
        <section className="comparison-card" aria-labelledby="comparison-heading">
          <h3 id="comparison-heading">Implementations</h3>
          <div className="table-scroll" role="region" aria-label="Implementation comparison" tabIndex={0}>
            <table><thead><tr><th scope="col">Implementation</th><th scope="col">Checks</th>{rule?.metrics?.map((m) => <th scope="col" key={m.name}>{m.name} ({m.unit})</th>)}<th scope="col">Decision</th></tr></thead>
              <tbody>{candidates.map((c) => <tr key={c.id} className={c.id === selected?.candidate ? "selected-row" : ""}>
                <th scope="row">{title(c)}{c.parents?.length > 0 && <small>From {c.parents.map((id) => title(all.find((item) => item.id === id))).join(" + ")}</small>}</th><td><span className={`state ${c.state}`}>{c.state}</span>{c.detail && <small>{c.detail}</small>}</td>
                {rule?.metrics?.map((m) => <td key={m.name}>{c.score?.[m.name] ? <>{number(c.score[m.name].value)}
                  <small>{number(c.score[m.name].min)}–{number(c.score[m.name].max)} · {c.score[m.name].n} samples</small></> : "—"}</td>)}
                <td>
                  {c.id === selected?.candidate ? (selected.state === "landed" ? "Landed" : "Selected") :
                    c.id === selected?.recommended ? "Recommended" :
                    c.baseline ? "Baseline" : c.state === "passed" && c.eligible === false ? "Doesn’t qualify" : ""}
                  {canDecide && !c.baseline && c.state === "passed" && c.eligible !== false && c.id !== selected?.candidate &&
                    <button disabled={busy} onClick={() => act({ kind: "select", candidate: c.id })}>Select</button>}
                </td>
              </tr>)}</tbody></table>
          </div>
        </section>
        <Charts {...{ run, candidates, rule }} policy={latestPolicy} />
      </div>
    </> : <p>Measurements appear as candidates finish.</p>}
    <div className="actions">
      {canDecide && <button disabled={busy} onClick={() => act({ kind: "keep-baseline" })}>Keep current version</button>}
      {canDecide && winner?.eligible && !winner.baseline && ["selected", "recommended"].includes(selected?.state) && (!run.input.delivery || run.input.delivery === "review" || run.held || !run.repo.Target) &&
        <>{!run.repo.Target && <label>Target branch<input value={target} onChange={(event) => setTarget(event.target.value)} /></label>}
          <button className="primary" disabled={busy || landingBlocked || (!run.repo.Target && !target.trim())}
            onClick={() => act({ kind: "land", candidate: winner.id, text: target })}>
            {run.repo.Target ? `Land selected on ${branchName(run.repo.Target)}` : "Land selected implementation"}
          </button></>}
      {run.state !== "landed" && run.input.delivery && run.input.delivery !== "review" &&
        <button disabled={busy || !!run.combination} onClick={() => act({ kind: run.held ? "unhold" : "hold" })}>{run.held ? "Resume auto-land" : "Hold auto-land"}</button>}
      {isCurrent && implemented.length > 1 && (canDecide || run.state === "landed") &&
        <button className={run.state === "landed" ? "primary" : undefined} aria-expanded={mode === "combine"} disabled={busy || !!run.combination} onClick={() => { setBase((!winner?.baseline && winner?.id) || current[0]?.id || ""); setDonors([]); setMode(mode === "combine" ? "" : "combine"); }}>
          {run.state === "landed" ? "Improve this version" : "Combine aspects"}
        </button>}
      {canDecide && run.input.recipe.rubric && previewable.length > 1 &&
        <button onClick={() => { setBase(previewable[0].id); setDonors([previewable[1].id]); setMode("blind"); }}>Blind review</button>}
      {isCurrent && (canDecide || run.state === "landed") &&
        <button aria-expanded={mode === "next"} disabled={busy} onClick={() => { setMode(mode === "next" ? "" : "next"); setGoal(""); }}>Start another round</button>}
    </div>
    {canDecide && run.input.recipe.rubric && <p>Review criteria · {run.input.recipe.rubric}</p>}
    {mode === "next" && <form className="answer" onSubmit={(event) => {
      event.preventDefault(); act({ kind: "new-round", text: goal }).then((ok) => { if (ok) setMode(""); });
    }}>
      <label>What should the swarm try next?<textarea required value={goal} onChange={(event) => setGoal(event.target.value)} /></label>
      <button className="primary" disabled={busy || !goal.trim()}>Start round</button>
    </form>}
    {mode === "combine" || mode === "blind" ? <form className="answer" onSubmit={(event) => {
      event.preventDefault();
      act({ kind: mode === "combine" ? "combine" : "blind-review", candidate: base, donors, text: goal }).then((ok) => { if (ok) setMode(""); });
    }}>
      <label>Start from<select required value={base} disabled={run.state === "landed"} onChange={(event) => {
        setBase(event.target.value); setDonors(donors.filter((d) => d !== event.target.value));
      }}><option value="">Choose implementation</option>{(mode === "blind" ? previewable : implemented).map((c) => <option key={c.id} value={c.id}>{title(c)}</option>)}</select></label>
      <fieldset><legend>{mode === "combine" ? "Take aspects from" : "Compare against"}</legend>
        {(mode === "blind" ? previewable : implemented).filter((c) => c.id !== base).map((c) => <label key={c.id} className="checkbox">
          <input type={mode === "combine" ? "checkbox" : "radio"} name="donors" checked={donors.includes(c.id)} onChange={(event) =>
            setDonors(mode === "blind" ? [c.id] : event.target.checked ? [...donors, c.id] : donors.filter((id) => id !== c.id))} />{title(c)}</label>)}
      </fieldset>
      {mode === "combine" && <label>What should carry over?<textarea required value={goal} onChange={(event) => setGoal(event.target.value)} /></label>}
      <button className="primary" disabled={busy || !base || !donors.length || mode === "combine" && !goal.trim()}>{mode === "combine" ? "Build combination" : "Start review"}</button>
      <button type="button" onClick={() => setMode("")}>Cancel</button>
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
            <button disabled={busy} key={side} onClick={() => perform(async () => {
              const { url } = await api(`/runs/${run.id}/blind/${p.id}/${side}`, {});
              setAB((current) => ({ ...current, [key]: url }));
            })}>Load {side.toUpperCase()}</button>;
        })}</div>
        <div className="actions">{[["left", "Prefer A"], ["right", "Prefer B"], ["tie", "Tie"], ["skip", "Skip"]].map(([value, label]) =>
          <button key={value} disabled={busy} onClick={() => act({ kind: "vote", task: p.id, text: value })}>{label}</button>)}</div></>}
    </article>)}
    <div className="actions experiment-tools">
      <button onClick={() => setDetail("suite")}>Eval suite{run.suite_proposal ? " · proposal ready" : ""}</button>
      <button onClick={() => { setDetail("cases"); if (!savedCases) loadCases(); }}>Project eval cases</button>
      <button onClick={() => { setDetail("history"); if (!history) loadHistory(); }}>Project run history</button>
    </div>
    {detail === "suite" && <Dialog title="Eval suite" onClose={() => setDetail("")}>
      <RecipeEditor key={run.policy} recipe={run.suite_proposal || run.input.recipe} label="Suite" submitLabel="Approve suite revision"
        busy={busy || !["review", "working", "blocked"].includes(run.state)}
        onApprove={async (recipe) => { const ok = await act({ kind: "update-suite", recipe }); if (ok) setDetail(""); return ok; }} />
    </Dialog>}
    {detail === "cases" && <Dialog title="Project eval cases" onClose={() => setDetail("")}>
      {!savedCases && (busy ? <p role="status">Loading cases…</p> : <button onClick={() => loadCases()}>Reload cases</button>)}
      {savedCases && !savedCases.length && <p>No saved cases for this project.</p>}
      {savedCases?.map((entry) => <article className="row" key={entry.cursor}><strong>{entry.scenario}</strong>
        <p>Expected · {entry.expected}</p><Markdown>{entry.why}</Markdown><small>{entry.active ? "Active in this suite" : entry.state} · Revision {entry.revision} · {entry.author}</small></article>)}
      {savedCases?.length >= 100 && savedCases.length % 100 === 0 && <button onClick={() => loadCases(savedCases.at(-1).cursor)}>Load older revisions</button>}
    </Dialog>}
    {detail === "history" && <Dialog title="Project run history" onClose={() => setDetail("")}>
      {!history && (busy ? <p role="status">Loading history…</p> : <button onClick={() => loadHistory()}>Reload history</button>)}
      <ul>{history?.map((entry) => <li key={entry.id}><button disabled={entry.id === run.id}
        onClick={() => openRun(entry.id)}>{runTitle(entry)} · {entry.state}</button></li>)}</ul>
      {history?.length >= 20 && history.length % 20 === 0 && <button onClick={() => loadHistory(history.at(-1).cursor)}>Load older runs</button>}
    </Dialog>}
    <form className="form-card" onSubmit={(event) => {
      event.preventDefault(); act({ kind: "feedback", text: feedback, candidate: winner?.id, task: rollout ? "rollout" : "" })
        .then((ok) => { if (ok) setFeedback(""); });
    }}>
      <label>Feedback for the coordinator<textarea value={feedback} onChange={(event) => setFeedback(event.target.value)} /></label>
      <label className="checkbox"><input type="checkbox" checked={rollout} onChange={(event) => setRollout(event.target.checked)} />From a live rollout</label>
      <button disabled={!feedback.trim() || busy}>Send feedback</button>
    </form>
  </section>;
}

const releaseLabels = {
  queued: "Queued", building: "Building artifact", built: "Artifact verified", "reading-baseline": "Checking destination",
  staged: "Ready to deploy", deploying: "Deploying", verify: "Awaiting live check", verifying: "Checking live artifact",
  "needs-rollback": "Rollback needed", "rolling-back": "Rolling back", "verify-rollback": "Awaiting rollback check",
  "checking-rollback": "Checking rollback", uncertain: "Needs live check", reconcile: "Awaiting live check",
  reconciling: "Checking live state", deployed: "Live and healthy", failed: "Failed", "rolled-back": "Rolled back",
  superseded: "Superseded",
};

export function ReleaseList({ run, act, busy }) {
  if (!run.releases?.length) return null;
  return <section>
    <h2>Releases</h2>
    {run.releases.map((release) => <article key={release.id} className="row">
      <header><strong>{release.destination}</strong><span className={`state ${release.state === "deployed" ? "passed" : release.state === "failed" ? "failed" : release.state === "uncertain" || release.held ? "question" : ""}`}>
        {release.held ? "On hold" : releaseLabels[release.state] || release.state}
      </span></header>
      <p><code>{release.commit}</code></p>
      {release.digest && <p>Artifact <code>{release.digest}</code></p>}
      {release.observed && <p>Live artifact <code>{release.observed}</code></p>}
      {release.detail && <p>{release.detail}</p>}
      {release.at && <p>{when(release.at)}</p>}
      {["queued", "building", "built", "reading-baseline", "staged"].includes(release.state) &&
        <button disabled={busy} onClick={() => act({ kind: release.held ? "resume-release" : "hold-release", task: release.id })}>{release.held ? "Resume release" : "Hold release"}</button>}
      {release.state === "uncertain" && <button disabled={busy} onClick={() => act({ kind: "reconcile-release", task: release.id })}>Check live state</button>}
    </article>)}
  </section>;
}
