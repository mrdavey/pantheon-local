import { Button } from "./components/ui/button";
import React, { useState } from "react";
import { Dialog } from "./dialog.jsx";
import { runTitle, overTokenTarget } from "./run-summary.js";

export function RunSettings({ run, send, remove, busy, error, onClose }) {
  const [confirm, setConfirm] = useState(false);
  const paused = run.state === "stopped";
  const landed = run.state === "landed";
  const overBudget = overTokenTarget(run);
  const active = run.turns?.some(turn => turn.state === "running") || Boolean(run.landing);
  const pendingRelease = run.releases?.some(release => !["deployed", "rolled-back", "failed", "superseded", "cancelled"].includes(release.state));
  const reason = pendingRelease ? "Finish or reconcile the release first." : !paused && !landed ? "Pause the run before deleting it." : active ? "Waiting for active work to stop." : "";
  return <Dialog title={confirm ? "Delete this run?" : "Run settings"} onClose={onClose}>
    <p className="settings-title">{runTitle(run)}</p>
    {error && <p role="alert" className="notice error">{error}</p>}
    {confirm ? <>
      <p>This permanently removes the run history, logs, and private checkouts. Your project folder and landed commits stay on disk.</p>
      <div className="actions"><Button variant="outline" autoFocus onClick={() => setConfirm(false)}>Cancel</Button>
        <Button variant="outline" className="danger" disabled={busy || !!reason} onClick={remove}>Delete permanently</Button></div>
    </> : <>
      <dl className="settings-details"><dt>Coordinator</dt><dd>{run.input.coordinator}</dd>
        <dt>Workers</dt><dd>{run.input.workers.map(name => {
          const p = run.profiles?.[name];
          return <div key={name}>{[name, p?.model || p?.detected?.label, p?.effort || p?.detected?.effort].filter(Boolean).join(" · ")}</div>;
        })}</dd>
        <dt>Subagents</dt><dd>{run.input.subagents === false ? "Off" : run.input.subagent_limit ? `Up to ${run.input.subagent_limit} at once` : "Unlimited"}</dd>
        <dt>Target branch</dt><dd>{run.repo?.Target || "Current branch"}</dd>
        <dt>Delivery</dt><dd>{({review: "Review before landing", auto: "Auto-land", deploy: "Auto-deploy"})[run.input.delivery || "review"] || run.input.delivery}</dd></dl>
      {(!landed || overBudget) && <div className="actions">
        {(["stopped", "blocked", "budget"].includes(run.state) || overBudget) && <Button variant="outline" disabled={busy || (paused && active)} onClick={() => send(run, {kind: "resume", tokens: overBudget ? 0 : run.input.token_target})}>{overBudget ? "Continue without target" : "Resume"}</Button>}
        {!landed && !paused && <Button variant="outline" disabled={busy} onClick={() => send(run, {kind: "stop"})}>Pause run</Button>}
        {!landed && !!run.input.delivery && run.input.delivery !== "review" && <Button variant="outline" disabled={busy} onClick={() => send(run, {kind: "delivery", text: "review"})}>Require review</Button>}
      </div>}
      <div className="settings-delete"><Button variant="outline" className="danger" disabled={busy || !!reason} onClick={() => setConfirm(true)}>Delete run</Button></div>
    </>}
    {reason && <p className="muted">{reason}</p>}
  </Dialog>;
}
