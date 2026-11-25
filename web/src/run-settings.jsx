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
      <div className="actions"><button autoFocus onClick={() => setConfirm(false)}>Cancel</button>
        <button className="danger" disabled={busy || !!reason} onClick={remove}>Delete permanently</button></div>
    </> : <>
      <dl className="settings-details"><dt>Coordinator</dt><dd>{run.input.coordinator}</dd>
        <dt>Target branch</dt><dd>{run.repo?.Target || "Current branch"}</dd>
        <dt>Delivery</dt><dd>{({review: "Review before landing", auto: "Auto-land", deploy: "Auto-deploy"})[run.input.delivery || "review"] || run.input.delivery}</dd></dl>
      {(!landed || overBudget) && <div className="actions">
        {(["stopped", "blocked", "budget"].includes(run.state) || overBudget) && <button disabled={busy || (paused && active)} onClick={() => send(run, {kind: "resume", tokens: overBudget ? 0 : run.input.token_target})}>{overBudget ? "Continue without target" : "Resume"}</button>}
        {!landed && !paused && <button disabled={busy} onClick={() => send(run, {kind: "stop"})}>Pause run</button>}
        {!landed && !!run.input.delivery && run.input.delivery !== "review" && <button disabled={busy} onClick={() => send(run, {kind: "delivery", text: "review"})}>Require review</button>}
      </div>}
      <div className="settings-delete"><button className="danger" disabled={busy || !!reason} onClick={() => setConfirm(true)}>Delete run</button></div>
    </>}
    {reason && <p className="muted">{reason}</p>}
  </Dialog>;
}
