import React, { useCallback, useEffect, useRef, useState } from "react";

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
    installed: "Set the provider and model in Hermes Desktop.",
  },
  antigravity: {
    name: "Antigravity",
    docs: "https://antigravity.google/docs/cli-getting-started",
    command: "agy",
    installed: "Finish setup and sign in in Antigravity.",
  },
};

const labels = {
  connected: "Signed in", installed: "Installed", missing: "Install required",
  "sign-in": "Sign in required", "sign-in-unknown": "Sign-in unverified", invalid: "Invalid profile",
};

const rank = {connected: 0, installed: 1, "sign-in-unknown": 2, "sign-in": 3, missing: 4, invalid: 5};

export function profileLabel(name, profile) {
  if (profile?.kind === "hermes" && profile.hermes_profile && (name === "hermes" || name === `hermes-${profile.hermes_profile}`)) {
    return `Hermes · ${profile.hermes_profile}`;
  }
  return name === profile?.kind ? agents[name]?.name || name : name;
}

function BenchmarkSettings({ name, profile, save }) {
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
  return <details>
    <summary>Model and benchmark</summary>
    <form onSubmit={submit}>
      {passesModel ? <label>Model ID<input value={model} onChange={(event) => setModel(event.target.value)} placeholder="Use the CLI default" /></label> : <p>Choose the runtime model in your native agent.</p>}
      <label>Model name on the benchmark<input value={benchmarkModel} onChange={(event) => setBenchmarkModel(event.target.value)} placeholder={passesModel ? "Use model ID if it matches" : "Enter the exact displayed name"} /></label>
      <div className="options">
        <label>Agent name on the benchmark<input value={agent} onChange={(event) => setAgent(event.target.value)} placeholder="Optional for known agents" /></label>
        <label>Benchmark effort<input value={effort} onChange={(event) => setEffort(event.target.value)} placeholder="Optional" /></label>
      </div>
      <p>Match the model and harness you actually use. <a href="https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/5?tab=leaderboard" target="_blank" rel="noreferrer">View the Terminal-Bench leaderboard</a>.</p>
      <button type="submit" disabled={saving}>{saving ? "Saving model settings…" : "Save model settings"}</button>
      {status && <p role={status.error ? "alert" : "status"}>{status.text}</p>}
    </form>
  </details>;
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

export function AgentSetup({ check, saveModel, onContinue }) {
  const [report, setReport] = useState(null);
  const [checking, setChecking] = useState(true);
  const [error, setError] = useState("");
  const request = useRef(null);
  const [choice, setChoice] = useState("");
  const refresh = useCallback(async () => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    setChecking(true);
    setError("");
    try {
      const next = await check(controller.signal);
      if (!controller.signal.aborted) setReport(next);
    } catch (problem) {
      if (!controller.signal.aborted) setError(problem.message);
    } finally {
      if (!controller.signal.aborted) setChecking(false);
    }
  }, [check]);
  useEffect(() => {
    refresh();
    return () => request.current?.abort();
  }, [refresh]);

  async function updateModel(input) {
    const setup = await saveModel(input);
    setReport((value) => ({ ...value, setup }));
  }

  const profiles = Object.entries(report?.setup.profiles || {});
  profiles.sort(([a], [b]) => (rank[report.profiles[a]?.state] ?? 9) -
    (rank[report.profiles[b]?.state] ?? 9) || a.localeCompare(b));

  const usable = ([name]) => ["connected", "installed", "sign-in-unknown"].includes(report.profiles[name]?.state);
  const available = profiles.filter(usable);
  const others = profiles.filter((p) => !usable(p));
  const selected = available.find(([name]) => name === choice) || available[0];
  const stateOf = (name) => report.profiles[name]?.state;
  const agent = selected && agents[selected[1].kind];
  const needsLoginCheck = selected && stateOf(selected[0]) === "sign-in-unknown";

  const setupSteps = <ul className="setup-list">{others.map(([name, profile]) => {
    const state = stateOf(name);
    const info = agents[profile.kind];
    return <li className="setup-step" key={name}>
      <strong>{profileLabel(name, profile)}</strong>
      {state === "missing" && info
        ? <a href={info.docs} target="_blank" rel="noreferrer">Install {info.name}</a>
        : state === "sign-in" && info?.command
          ? <><span>{labels[state]}</span><SetupCommand command={info.command} /></>
          : <p>Check the executable in <code>profiles.json</code>.</p>}
    </li>;
  })}</ul>;

  return <section className="panel setup">
    <header className="page-heading">
      <h1 tabIndex={-1}>Agent setup</h1>
      <button aria-disabled={checking} onClick={() => { if (!checking) refresh(); }}>
        {checking ? "Checking agents…" : "Check again"}
      </button>
    </header>
    {checking && <p role="status">Checking local agents…</p>}
    {error && <p className="notice" role="alert">Couldn’t check agents. {error}</p>}
    {report && <>
      {!report.git && <p className="notice" role="alert">Git is required. <a href="https://git-scm.com/install/" target="_blank" rel="noreferrer">Install Git</a> and check again.</p>}
      {!profiles.length && <p>No profiles found. Run <code>swarm doctor</code> to locate <code>profiles.json</code>.</p>}
      {selected && <div className="form-card agent-choice">
        <fieldset><legend>Choose an agent</legend>
          {available.map(([name, profile]) => <label key={name} className="agent-option">
            <input type="radio" name="agent-choice" aria-label={profileLabel(name, profile)} aria-describedby={`status-${encodeURIComponent(name)}`} checked={name === selected[0]} onChange={() => setChoice(name)} />
            <span>{profileLabel(name, profile)}</span>
            <small id={`status-${encodeURIComponent(name)}`} className={"state " + (stateOf(name) === "connected" ? "passed" : "")}>{labels[stateOf(name)]}</small>
          </label>)}
        </fieldset>
        {selected[1].kind === "hermes" && <p>{agent.installed} Runs use a separate session in this profile.</p>}
        {stateOf(selected[0]) === "installed" && selected[1].kind === "antigravity" && <p>{agent.installed}</p>}
        {needsLoginCheck && <p>Check your sign-in before starting.</p>}
        {needsLoginCheck && agent?.command && <SetupCommand command={agent.command} />}
        <button className="primary" disabled={checking || !report.git} onClick={() => onContinue(selected[0], report.setup)}>
          Use {profileLabel(selected[0], selected[1])}
        </button>
        <BenchmarkSettings key={selected[0]} name={selected[0]} profile={selected[1]} save={updateModel} />
      </div>}
      {others.length > 0 && (available.length
        ? <details className="setup-more"><summary>Set up another agent ({others.length})</summary>{setupSteps}</details>
        : <div className="form-card"><h2>Set up your first agent</h2>{setupSteps}<p>After setup, select Check again.</p></div>)}
    </>}
  </section>;
}
