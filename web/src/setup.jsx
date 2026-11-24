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
  const [draft, setDraft] = useState({});
  const [status, setStatus] = useState(null);
  const [saving, setSaving] = useState(false);
  const detected = profile.detected || {};
  const passesModel = ["codex", "claude"].includes(profile.kind);
  const auto = {
    model: detected.model || "",
    benchmark_model: detected.routing?.model || "",
    benchmark_agent: detected.routing?.agent || "",
    benchmark_effort: detected.effort || "",
  };
  const value = (key) => Object.hasOwn(draft, key) ? draft[key] ?? auto[key] : profile[key] || auto[key];
  const modelChanged = value("model") !== (profile.model || auto.model);
  if (modelChanged) {
    auto.benchmark_model = value("model");
    auto.benchmark_effort = "";
  }
  const editable = (key) => ({
    value: value(key), disabled: saving, autoComplete: "off", maxLength: 150,
    onChange: (e) => { setStatus(null); setDraft({ ...draft, [key]: e.target.value }); },
  });
  async function submit(event) {
    event.preventDefault();
    setSaving(true);
    setStatus(null);
    try {
      const input = { profile: name };
      for (const key of Object.keys(auto)) {
        // Preserve explicit choices but never turn an untouched native default into a pin.
        input[key] = Object.hasOwn(draft, key) ? (draft[key] || "").trim() : profile[key] || "";
      }
      await save(input);
      setDraft({});
      setStatus({ text: "Saved for new runs." });
    } catch (problem) {
      setStatus({ error: true, text: problem.message });
    } finally {
      setSaving(false);
    }
  }
  const modelName = modelChanged ? value("model") : detected.label || profile.model || detected.model;
  const manualModel = passesModel && !!(Object.hasOwn(draft, "model") ? draft.model : profile.model);
  return <Dialog title={`Settings for ${profileLabel(name, profile)}`} onClose={onClose}>
    <form onSubmit={submit}>
      <div className="model-identity">
        <h3>{modelName || "Agent default"}</h3>
        {modelName && <small>{manualModel ? "Pantheon model override" : detected.source}</small>}
        {!modelChanged && detected.effort && <span title={detected.effort_source}>{detected.effort} reasoning</span>}
      </div>
      {passesModel ? <label>Model ID<input {...editable("model")} placeholder="Use the CLI default" /></label>
        : <label>Model ID<input readOnly value={detected.model || ""} placeholder="Not reported by this agent" /></label>}
      {detected.alias && !modelChanged && <p>Claude Code resolves this alias when a session starts.</p>}
      <p>{modelHint(profile)}</p>
      <h3>Benchmark routing</h3>
      {!Object.keys(draft).length && (detected.routing?.source
        ? <p className="model-provenance">Matched {detected.routing.source}.</p>
        : <p className="model-provenance">No matching local benchmark result.</p>)}
      <label>Model name on the benchmark<input {...editable("benchmark_model")} placeholder="Benchmark model name" /></label>
      <div className="options">
        <label>Agent name on the benchmark<input {...editable("benchmark_agent")} placeholder="Benchmark agent name" /></label>
        <label>Benchmark effort<input {...editable("benchmark_effort")} placeholder="Optional" /></label>
      </div>
      <p><a href="https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard" target="_blank" rel="noreferrer">View Terminal-Bench results</a></p>
      <div className="agent-actions">
        <button className="primary" type="submit" disabled={saving}>{saving ? "Saving model settings…" : "Save model settings"}</button>
        {(Object.keys(auto).some((key) => !!profile[key]) || Object.keys(draft).length > 0) &&
          <button type="button" disabled={saving} onClick={() => {
            setDraft({ model: null, benchmark_model: null, benchmark_agent: null, benchmark_effort: null });
            setStatus(null);
          }}>Use detected settings</button>}
      </div>
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
        const info = agents[profile?.kind];
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
