import { Disclosure } from "./disclosure";
import { Button } from "./components/ui/button";
import { Textarea } from "./components/ui/textarea";
import { Label } from "./components/ui/label";
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

function CheckSummary({ argv, preview = false }) {
  if (!Array.isArray(argv) || !argv.length) return null;
  const command = [...argv];
  if (["env", "/usr/bin/env"].includes(command[0]) && /^PATH=/.test(command[1])) command.splice(0, 2);
  const [program, ...args] = command;
  const name = program?.split("/").at(-1);
  const scripts = { lint: "Check code style", typecheck: "Check types", test: "Run tests", build: "Build the project", "test:integration": "Run integration tests" };
  let summary;
  if (["npm", "yarn", "pnpm"].includes(name)) {
    const script = args.length === 2 && args[0] === "run" ? args[1] : args.length === 1 ? args[0] : "";
    summary = Object.hasOwn(scripts, script) ? scripts[script] : name === "npm" && script === "ci" ? "Install locked dependencies" : null;
  }
  if (preview) summary = "Start the local preview";
  if (!summary) return <Command argv={argv} />;
  return <Disclosure className="check-command" title={summary}><Command argv={argv} /></Disclosure>;
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
      {recipe.checks.map((command, index) => <li key={index}><CheckSummary argv={command} /></li>)}
    </ul>}
    {draft === null && recipe.rule && <SelectionRule rule={recipe.rule} />}
    {draft === null && recipe.cases?.length > 0 && <Disclosure title={<>{recipe.cases.length} measured {recipe.cases.length === 1 ? "case" : "cases"}</>}>
      <ul className="check-list">{recipe.cases.map((entry, index) => <li key={index}>
        <strong>{entry.id}</strong> · {entry.samples || 1} {(entry.samples || 1) === 1 ? "sample" : "samples"}
        {entry.required ? " · Required" : ""}{entry.holdback ? " · Held back" : ""}
        {entry.program ? <pre>{entry.program}</pre> : <p><code>{entry.driver}</code></p>}
      </li>)}</ul>
    </Disclosure>}
    {draft === null && recipe.preview?.length > 0 && <CheckSummary argv={recipe.preview} preview />}
    {draft === null && recipe.rubric && <p>Review criteria · {recipe.rubric}</p>}
    <Disclosure open={editing} onOpenChange={setEditing} title="View or edit JSON">
      <Label>{label}<Textarea rows={8} spellCheck={false} aria-invalid={invalid || undefined}
        value={draft ?? JSON.stringify(recipe, null, 2)} onChange={(event) => {
          setDraft(event.target.value);
          setInvalid(false);
        }} /></Label>
      {invalid && <p role="alert">Enter a valid JSON object to approve these checks.</p>}
    </Disclosure>
    <Button variant="outline" type="submit" className="primary" disabled={busy}>{submitLabel}</Button>
  </form>;
}
