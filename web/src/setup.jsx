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
    command: "hermes model",
    installed: "Select a provider and model in Hermes.",
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

  return <section className="panel setup">
    <header className="page-heading">
      <h1 tabIndex={-1}>Connect an agent</h1>
      <button aria-disabled={checking} onClick={() => { if (!checking) refresh(); }}>
        {checking ? "Checking agents…" : "Check again"}
      </button>
    </header>
    <p>One agent can coordinate and work in separate sessions.</p>
    {checking && <p role="status">Checking for Git and local agent profiles.</p>}
    {error && <p className="notice" role="alert">Couldn’t check agents. {error}</p>}
    {report && <>
      {!report.git && <p className="notice" role="alert">Git is required. <a href="https://git-scm.com/install/" target="_blank" rel="noreferrer">Install Git</a> and check again.</p>}
      {!profiles.length && <p>No agent profiles found. Run <code>swarm doctor</code> to find your <code>profiles.json</code> file and add a profile.</p>}
      <ul className="setup-list">
        {profiles.map(([name, profile]) => {
          const agent = agents[profile.kind];
          const state = report.profiles[name]?.state;
          const usable = ["connected", "installed", "sign-in-unknown"].includes(state);
          return <li className="row setup-card" key={name}>
            <header>
              <strong>{agent?.name || name}{agent && name !== profile.kind ? <small className="profile-name">{name}</small> : ""}</strong>
              <span className={"state " + (state === "connected" ? "passed" : usable && state !== "sign-in-unknown" ? "" : "question")}>{labels[state] || "Unchecked"}</span>
            </header>
            {state === "missing" && agent && <p><a href={agent.docs} target="_blank" rel="noreferrer">Install {agent.name}</a>, then check again.</p>}
            {state === "missing" && !agent && <p>Check the executable path in <code>profiles.json</code>.</p>}
            {state === "sign-in" && <p>Run this in Terminal, then check again.</p>}
            {state === "sign-in-unknown" && <p>Check your sign-in in the agent’s CLI before starting.</p>}
            {state === "installed" && agent?.installed && <p>{agent.installed}{profile.hermes_profile && <> Use the <strong>{profile.hermes_profile}</strong> profile.</>}</p>}
            {state === "installed" && !agent && <p>Confirm any required account setup in the agent’s CLI.</p>}
            {["sign-in", "sign-in-unknown", "installed"].includes(state) && agent?.command &&
              <SetupCommand command={agent.command} />}
            {state === "invalid" && <p>Check the profile in <code>profiles.json</code>.</p>}
            {usable && <>
              <div className="actions"><button className={state === "connected" ? "primary" : ""} disabled={checking || !report.git} onClick={() => onContinue(name, report.setup)}>Use {name}</button></div>
              <BenchmarkSettings {...{ name, profile }} save={updateModel} />
            </>}
          </li>;
        })}
      </ul>
    </>}
  </section>;
}
