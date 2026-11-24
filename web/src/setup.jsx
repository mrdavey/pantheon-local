import React, { useState } from "react";
import { Dialog } from "./dialog.jsx";
import "./setup.css";

const agents = {
  codex: {
    name: "Codex",
    docs: "https://developers.openai.com/codex/cli/",
    command: "codex login",
  },
  claude: {
    name: "Claude Code",
    docs: "https://code.claude.com/docs/en/quickstart",
    command: "claude auth login",
  },
  hermes: {
    name: "Hermes",
    docs: "https://hermes-agent.nousresearch.com/docs/getting-started/quickstart",
  },
  antigravity: {
    name: "Antigravity",
    docs: "https://antigravity.google/docs/cli-getting-started",
    command: "agy",
  },
};

const labels = {
  connected: "Signed in", installed: "Installed", missing: "Install required",
  "sign-in": "Sign in required", "sign-in-unknown": "Sign-in unverified", invalid: "Invalid profile",
};

const usableStates = ["connected", "installed", "sign-in-unknown"];

export function profileLabel(name, profile) {
  if (profile?.kind === "hermes" && profile.hermes_profile && (name === "hermes" || name === `hermes-${profile.hermes_profile}`)) {
    return `Hermes · ${profile.hermes_profile}`;
  }
  return name === profile?.kind ? agents[name]?.name || name : name;
}

export function availableNames(report) {
  return Object.keys(report?.setup.profiles || {}).filter((name) => usableStates.includes(report.profiles[name]?.state))
    .sort((a, b) => usableStates.indexOf(report.profiles[a].state) - usableStates.indexOf(report.profiles[b].state) || a.localeCompare(b));
}

export function modelHint(profile) {
  switch (profile?.kind) {
    case "codex": return "Use /model in Codex to check or change the default model and reasoning level.";
    case "claude": return "Use /model and /effort in Claude Code to check or change the defaults.";
    case "hermes": return "Choose the model and reasoning level in your Hermes profile.";
    case "antigravity": return "Choose the model in Antigravity.";
    default: return "This adapter controls its model and reasoning settings.";
  }
}

export function ModelSettings({ name, profile, save, onClose }) {
  const [model, setModel] = useState(profile.model || "");
  const [benchmarkModel, setBenchmarkModel] = useState(profile.benchmark_model || "");
  const [agent, setAgent] = useState(profile.benchmark_agent || "");
  const [effort, setEffort] = useState(profile.benchmark_effort || "");
  const [status, setStatus] = useState(null);
  const [saving, setSaving] = useState(false);
  const passesModel = profile.kind === "codex" || profile.kind === "claude";
  async function submit(event) {
    event.preventDefault();
    setSaving(true);
    setStatus(null);
    try {
      await save({ profile: name, model, benchmark_model: benchmarkModel, benchmark_agent: agent, benchmark_effort: effort });
      setStatus({ text: "Saved for new runs." });
    } catch (problem) {
      setStatus({ error: true, text: problem.message });
    } finally {
      setSaving(false);
    }
  }
  return <Dialog title={`Settings for ${profileLabel(name, profile)}`} onClose={onClose}>
    <form onSubmit={submit}>
      {passesModel ? <label>Model ID<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Use the CLI default" /></label> : null}
      <p>{modelHint(profile)}</p>
      <h3>Benchmark routing</h3>
      <label>Model name on the benchmark<input value={benchmarkModel} onChange={(event) => setBenchmarkModel(event.target.value)} placeholder={passesModel ? "Use model ID if it matches" : "Enter the exact displayed name"} /></label>
      <div className="options">
        <label>Agent name on the benchmark<input value={agent} onChange={(event) => setAgent(event.target.value)} placeholder="Optional for known agents" /></label>
        <label>Benchmark effort<input value={effort} onChange={(event) => setEffort(event.target.value)} placeholder="Optional" /></label>
      </div>
      <p>Match the model and harness you actually use. <a href="https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard" target="_blank" rel="noreferrer">View the Terminal-Bench leaderboard</a>.</p>
      <button type="submit" disabled={saving}>{saving ? "Saving model settings…" : "Save model settings"}</button>
      {status && <p role={status.error ? "alert" : "status"}>{status.text}</p>}
    </form>
  </Dialog>;
}

function SetupCommand({ command }) {
  const [status, setStatus] = useState("");
  return <div className="setup-command">
    <code>{command}</code>
    {navigator.clipboard && <button aria-label={`Copy ${command}`} onClick={async () => {
      try {
        await navigator.clipboard.writeText(command);
        setStatus("Copied.");
      } catch {
        setStatus("Select the command to copy it.");
      }
    }}>Copy</button>}
    <span role="status">{status}</span>
  </div>;
}

export function AgentSetup({ report, checking, error, refresh, saveModel, onContinue }) {
  const [modal, setModal] = useState(null);
  const available = availableNames(report);
  const profiles = report?.setup.profiles || {};
  const others = Object.keys(profiles).filter((name) => !available.includes(name));
  const refreshButton = <button type="button" aria-disabled={checking} onClick={() => { if (!checking) refresh(); }}>
    {checking ? "Checking agents…" : "Check again"}
  </button>;
  const notices = <>
    {checking && <p role="status">Checking local agents…</p>}
    {error && <p className="notice" role="alert">Couldn’t check agents. {error}</p>}
  </>;
  return <section className="panel setup">
    <header className="page-heading"><h1 tabIndex={-1}>Agent setup</h1>{refreshButton}</header>
    {modal !== "new" && notices}
    {report && <>
      {!report.git && <p className="notice" role="alert">Git is required. <a href="https://git-scm.com/install/" target="_blank" rel="noreferrer">Install Git</a> and check again.</p>}
      <section className="form-card configured">
        <h2>Your configured agents</h2>
        {available.length ? <ul className="setup-list">{available.map((name) => {
          const profile = profiles[name];
          const label = profileLabel(name, profile);
          const state = report.profiles[name].state;
          return <li className="configured-row" key={name}>
            <div><strong>{label}</strong><small className={"state " + (state === "connected" ? "passed" : "")}>{labels[state]}</small></div>
            <div className="agent-actions">
              <button type="button" aria-label={`Settings for ${label}`} onClick={() => setModal({ name })}>Settings</button>
              <button type="button" aria-label={`Use ${label}`} disabled={checking || !report.git} onClick={() => onContinue(name, report.setup)}>Use</button>
            </div>
          </li>;
        })}</ul> : <p>No agents configured yet.</p>}
        <button type="button" className={available.length ? "" : "primary"} onClick={() => setModal("new")}>Set up new agent</button>
      </section>
    </>}
    {modal === "new" && <Dialog title="Set up new agent" onClose={() => setModal(null)}>
      {others.length ? <ul className="setup-list">{others.map((name) => {
        const profile = profiles[name];
        const state = report.profiles[name]?.state;
        const info = agents[profile.kind];
        return <li className="setup-step" key={name}>
          <strong>{profileLabel(name, profile)}</strong>
          {state === "missing" && info ? <a href={info.docs} target="_blank" rel="noreferrer">Install {info.name}</a>
            : state === "sign-in" && info?.command ? <><span>{labels[state]}</span><SetupCommand command={info.command} /></>
            : <p>Check the executable in <code>profiles.json</code>.</p>}
        </li>;
      })}</ul> : <p>All known profiles are configured. Add a profile in Hermes Desktop or <code>profiles.json</code> to connect another.</p>}
      {notices}
      <div className="dialog-footer">{refreshButton}<button type="button" className="primary" onClick={() => setModal(null)}>Done</button></div>
    </Dialog>}
    {modal?.name && profiles[modal.name] && <ModelSettings name={modal.name} profile={profiles[modal.name]} save={saveModel} onClose={() => setModal(null)} />}
  </section>;
}
