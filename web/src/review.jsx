import { Command } from "./evidence.jsx";
import React, { useState } from "react";

export function branchName(ref) {
  return ref?.replace(/^refs\/heads\//, "");
}

function SelectionRule({ rule }) {
  const metric = (name) => rule.metrics?.find((entry) => entry.name === name);
  const direction = (name) => metric(name)?.direction === "lower" ? "lower" : "higher";
  const unit = (name) => metric(name)?.unit || "";
  return <div className="recipe-rule">
    <p>Prefer {direction(rule.objective)} <strong>{rule.objective}</strong>{unit(rule.objective) && ` (${unit(rule.objective)})`}.</p>
    <ul className="check-list">
      {Object.entries(rule.limits || {}).map(([name, limit]) => <li key={name}>
        {name} must be {direction(name) === "lower" ? "at most" : "at least"} {limit} {unit(name)}
      </li>)}
      {rule.minimum > 0 && <li>Improve on the baseline by at least {rule.minimum} {unit(rule.objective)}</li>}
      {rule.min_lead > 0 && <li>Lead the next candidate by at least {rule.min_lead} {unit(rule.objective)}</li>}
      {rule.min_samples > 0 && <li>At least {rule.min_samples} samples per metric</li>}
      {rule.tie_break && <li>Break ties with {direction(rule.tie_break)} {rule.tie_break}</li>}
    </ul>
  </div>;
}

export function RecipeEditor({ recipe, busy, label, submitLabel, onApprove }) {
  const [draft, setDraft] = useState(null);
  const [invalid, setInvalid] = useState(false);
  const [editing, setEditing] = useState(false);
  return <form onSubmit={async (event) => {
    event.preventDefault();
    let value = recipe;
    if (draft !== null) {
      try {
        value = JSON.parse(draft);
        if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error();
      } catch {
        setEditing(true);
        setInvalid(true);
        return;
      }
    }
    if (await onApprove(value)) setDraft(null);
  }}>
    {draft !== null && <p>The edited JSON will be used when you approve.</p>}
    {draft === null && recipe.checks?.length > 0 && <ul className="check-list">
      {recipe.checks.map((command, index) => <li key={index}><Command argv={command} /></li>)}
    </ul>}
    {draft === null && recipe.rule && <SelectionRule rule={recipe.rule} />}
    {draft === null && recipe.cases?.length > 0 && <details>
      <summary>{recipe.cases.length} measured {recipe.cases.length === 1 ? "case" : "cases"}</summary>
      <ul className="check-list">{recipe.cases.map((entry, index) => <li key={index}>
        <strong>{entry.id}</strong> · {entry.samples || 1} {(entry.samples || 1) === 1 ? "sample" : "samples"}
        {entry.required ? " · Required" : ""}{entry.holdback ? " · Held back" : ""}
        {entry.program ? <pre>{entry.program}</pre> : <p><code>{entry.driver}</code></p>}
      </li>)}</ul>
    </details>}
    {draft === null && recipe.preview?.length > 0 && <p>Preview <Command argv={recipe.preview} /></p>}
    {draft === null && recipe.rubric && <p>Review criteria · {recipe.rubric}</p>}
    <details open={editing} onToggle={(event) => setEditing(event.currentTarget.open)}>
      <summary>View or edit JSON</summary>
      <label>{label}<textarea rows={8} spellCheck={false} aria-invalid={invalid || undefined}
        value={draft ?? JSON.stringify(recipe, null, 2)} onChange={(event) => {
          setDraft(event.target.value);
          setInvalid(false);
        }} /></label>
      {invalid && <p role="alert">Enter a valid JSON object to approve these checks.</p>}
    </details>
    <button type="submit" className="primary" disabled={busy}>{submitLabel}</button>
  </form>;
}
