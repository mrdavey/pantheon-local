import { Badge } from "./components/ui/badge";
import { Button } from "./components/ui/button";
import React, { useState } from "react";
import { Dialog } from "./dialog.jsx";
import { Markdown } from "./markdown.jsx";
import { shortSummary } from "./run-summary.js";

function taskName(id) { return id.replace(/[-_]+/g, " "); }

export function implementationLabel(candidate) {
  if (!candidate) return "Implementation";
  if (candidate.baseline) return "Baseline";
  const tasks = candidate.tasks || [];
  if (!candidate.name || candidate.name === tasks.join(" · ")) {
    if (tasks.length > 1) return "Combined implementation";
    return tasks.length ? taskName(tasks[0]) : "Implementation";
  }
  return shortSummary(candidate.name, 64);
}

export function ImplementationDetails({ candidate, run, rule, decision, onClose, explain }) {
  const tasks = (candidate?.tasks || []).map(id => run.tasks?.find(t => t.id === id) || { id });
  const name = candidate ? implementationLabel(candidate) : "Selection";
  const description = candidate?.name;
  const showDescription = description && description !== name && description !== candidate.tasks?.join(" · ") &&
    !tasks.some(task => task.goal === description);
  return <Dialog title={decision ? "Decision details" : "Implementation details"} placement="right" className="work-detail-dialog" onClose={onClose}>
    <div className="work-details">
      <section><h3>{name}</h3><div className="milestone-meta">
        {candidate && <Badge asChild variant="outline"><span className={`state ${candidate.state}`}>{candidate.state}</span></Badge>}
        {decision?.state && <span>{decision.state}</span>}
        {candidate?.commit && <code title={candidate.commit}>{candidate.commit.slice(0, 10)}</code>}
      </div>
        {candidate && <Button variant="outline" className="text-button explain-context" onClick={() => { onClose(); explain("implementation", candidate.id); }}>Explain this</Button>}
      </section>
      {decision?.reason && <section><h3>Why this decision?</h3><Markdown>{decision.reason}</Markdown></section>}
      {showDescription && <section><h3>Description</h3><Markdown>{description}</Markdown></section>}
      {candidate?.detail && <section><h3>Check result</h3><Markdown>{candidate.detail}</Markdown></section>}
      {tasks.length > 0 && <section><h3>Included milestones</h3><ul className="included-milestones">
        {tasks.map(t => <li key={t.id}><strong>{taskName(t.id)}</strong>
            {decision ? t.worker : t.goal && <Markdown>{t.goal}</Markdown>}</li>)}
      </ul></section>}
      {candidate?.parents?.length > 0 && <section><h3>Based on</h3><ul>{candidate.parents.map(id =>
        <li key={id}>{implementationLabel(run.candidates?.find(c => c.id === id))}</li>)}</ul></section>}
      {rule?.metrics?.some(m => candidate?.score?.[m.name]) && <section><h3>Measurements</h3><dl className="metric-details">
        {rule.metrics.filter(m => candidate.score[m.name]).map(m => {
          const score = candidate.score[m.name];
          return <div key={m.name}><dt>{m.name} ({m.unit})</dt>
            <dd>{score.value} · range {score.min}–{score.max} · {score.n} {score.n === 1 ? "sample" : "samples"}</dd></div>;
        })}
      </dl></section>}
    </div>
  </Dialog>;
}

export function ExperimentHelp() {
  const [open, setOpen] = useState(false);
  return <div className="implementation-help">
    <Button variant="outline" className="help-icon" aria-label="Help with experiment actions" title="Help with experiment actions" onClick={() => setOpen(true)}>?</Button>
    {open && <Dialog title="Experiment actions" placement="right" className="work-detail-dialog" onClose={() => setOpen(false)}>
      <dl className="experiment-glossary">
        <div><dt>Combine aspects or improve this version</dt><dd>Keep useful parts of different implementations. After landing, start from the version on your branch.</dd></div>
        <div><dt>Start another round</dt><dd>Give the coordinator a new goal to test against the current result.</dd></div>
        <div><dt>Eval suite</dt><dd>Review or revise the checks and measurements used to judge implementations.</dd></div>
        <div><dt>Compare case results</dt><dd>Inspect individual samples when scores differ or a check fails.</dd></div>
        <div><dt>Feedback</dt><dd>Tell the coordinator about a missed edge case or a result you saw after rollout.</dd></div>
      </dl>
    </Dialog>}
  </div>;
}
