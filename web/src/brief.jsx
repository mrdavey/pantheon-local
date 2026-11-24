import React, { useEffect, useState } from "react";
import { Dialog } from "./dialog.jsx";
import { availableNames, ModelSettings, profileLabel } from "./setup.jsx";

function ImportPicker({ api, path, selected, onChange, onClose }) {
  const [result, setResult] = useState(null);
  const [error, setError] = useState("");
  const [search, setSearch] = useState("");
  const [limit, setLimit] = useState(100);
  const [request, setRequest] = useState(0);
  const [files, setFiles] = useState(() => new Set(selected));
  useEffect(() => {
    const controller = new AbortController();
    setError("");
    api("/initial-files", { path }, controller.signal).then((data) => {
      if (!controller.signal.aborted) setResult(data);
    }).catch((e) => { if (!controller.signal.aborted) setError(e.message); });
    return () => controller.abort();
  }, [api, path, request]);
  const matches = result?.files.filter((name) => name.toLowerCase().includes(search.toLowerCase())) || [];
  return <Dialog title="Initial import files" onClose={onClose}>
    {error ? <><p role="alert">{error}</p><button onClick={() => setRequest(request + 1)}>Check again</button></>
      : !result ? <p role="status">Finding project files…</p>
      : <>
        <p><code>{result.root}</code></p>
        {!!result.files.length && <label>Find files<input type="search" value={search} onChange={(e) => { setSearch(e.target.value); setLimit(100); }} /></label>}
        <div className="import-files">{matches.slice(0, limit).map((name) =>
          <label key={name} className="checkbox"><input type="checkbox" checked={files.has(name)} onChange={({target}) => {
            const next = new Set(files);
            if (target.checked) next.add(name); else next.delete(name);
            setFiles(next);
          }} /><span>{name}</span></label>)}
          {!matches.length && <p>{search ? "No matching files." : "No files to import."}</p>}
          {matches.length > limit && <button onClick={() => setLimit(limit + 100)}>Show more files</button>}
        </div>
        <div className="dialog-footer">
          <span>{files.size} selected</span>
          <button className="primary" onClick={() => { onChange(result.files.filter((name) => files.has(name))); onClose(); }}>Use selected files</button>
        </div>
      </>}
  </Dialog>;
}

export function Brief({ setup, report, checking, checkError, cwd, saveModel, api, agentChoice, onSetup, onCreate, perform, busy }) {
  const [path, setPath] = useState(cwd);
  const [prompt, setPrompt] = useState("");
  const [suggestion, setSuggestion] = useState("");
  const [original, setOriginal] = useState("");
  const [coordinator, setCoordinator] = useState(agentChoice?.name || setup.coordinator || "");
  const [workers, setWorkers] = useState(agentChoice ? [agentChoice.name] : null);
  const [advisor, setAdvisor] = useState("");
  const [tokens, setTokens] = useState("");
  const [routing, setRouting] = useState("manual");
  const [initialFiles, setInitialFiles] = useState([]);
  const [target, setTarget] = useState("");
  const [compare, setCompare] = useState(false);
  const [delivery, setDelivery] = useState("auto");
  const [destination, setDestination] = useState("");
  const [destinations, setDestinations] = useState(null);
  const [destinationError, setDestinationError] = useState("");
  const [loadingDestinations, setLoadingDestinations] = useState(false);
  const [modal, setModal] = useState("");
  const [pickFiles, setPickFiles] = useState(false);
  useEffect(() => {
    if (agentChoice) {
      setCoordinator(agentChoice.name);
      setWorkers([agentChoice.name]);
    }
  }, [agentChoice]);
  const names = availableNames(report);
  const selectedCoordinator = names.includes(coordinator) ? coordinator : names[0] || "";
  const profile = setup.profiles?.[selectedCoordinator];
  const preferred = (setup.workers || []).filter((name) => names.includes(name));
  const selectedWorkers = workers === null ? (preferred.length ? preferred : names.slice(0, 1)) : workers.filter((name) => names.includes(name));
  const advisorName = names.includes(advisor) ? advisor : selectedCoordinator;
  const label = (name) => profileLabel(name, setup.profiles[name]);
  const changePath = (next) => { if (next !== path) { setPath(next); setInitialFiles([]); } };
  async function loadDestinations() {
    setLoadingDestinations(true);
    setDestinationError("");
    try {
      setDestinations(await api("/destinations"));
    } catch (problem) {
      setDestinationError(`Couldn’t load destinations. ${problem.message}`);
    } finally {
      setLoadingDestinations(false);
    }
  }
  const ready = !busy && report?.git && path.trim() && prompt.trim() && selectedCoordinator && selectedWorkers.length && (delivery !== "deploy" || destination);
  const create = (event) => {
    event.preventDefault();
    if (!ready) return;
    perform(async () => {
      const run = await api("/runs", {
        path, prompt, original_prompt: original, coordinator: selectedCoordinator, workers: selectedWorkers,
        routing: selectedWorkers.length > 1 ? routing : "manual", token_target: Number(tokens),
        initial_files: initialFiles, target, compare, delivery, destination: delivery === "deploy" ? destination : "", recipe: { checks: [] },
      });
      onCreate(run);
    }, "Starting run…");
  };
  return <section className="panel brief">
    <h1 tabIndex={-1}>Start a run</h1>
    <form className="form-card" onSubmit={create}>
      <label>What do you want to build?<textarea required rows={5} value={prompt} onChange={(event) => setPrompt(event.target.value)} /></label>
      <div className="brief-advice"><button type="button" disabled={busy || !prompt.trim() || !advisorName}
        onClick={() => perform(async () => setSuggestion(await api("/advice", { profile: advisorName, prompt })), "Improving brief…")}>Improve brief</button>
        {original && <button type="button" className="text-button" onClick={() => setModal("original")}>Original brief</button>}
      </div>
      {suggestion && <div className="answer">
        <label>Suggested brief<textarea rows={4} value={suggestion} onChange={(event) => setSuggestion(event.target.value)} /></label>
        <button type="button" onClick={() => { setOriginal(original || prompt); setPrompt(suggestion); setSuggestion(""); }}>Use suggestion</button>
        <button type="button" onClick={() => setSuggestion("")}>Dismiss</button>
      </div>}
      <div className="project-picker">
        <label htmlFor="project-folder">Project folder</label>
        <div className="path-controls">
          <input id="project-folder" required value={path} spellCheck={false} autoCapitalize="none" onChange={(event) => changePath(event.target.value)} placeholder="/path/to/project" />
          <button type="button" disabled={busy} onClick={() => perform(async () => {
            const folder = await api("/project-folder", {});
            if (folder) changePath(folder);
          }, "Choosing a folder…")}>Browse…</button>
        </div>
      </div>
      {names.length ? <div className="roles">
        <div><label>Coordinator<select aria-label="Coordinator" required title={label(selectedCoordinator)} value={selectedCoordinator} onChange={(event) => setCoordinator(event.target.value)}>
          {names.map((name) => <option key={name} value={name}>{label(name)}</option>)}</select></label>
          <button type="button" className="model-link text-button" onClick={() => setModal("model")}>
            {profile?.model || "Agent default"} · Model settings
          </button>
        </div>
        <fieldset><legend>Workers</legend>{names.map((name) => <label className="checkbox" key={name}><input type="checkbox" checked={selectedWorkers.includes(name)}
          onChange={(event) => setWorkers(event.target.checked ? [...selectedWorkers, name] : selectedWorkers.filter((worker) => worker !== name))} />{label(name)}</label>)}</fieldset>
      </div> : checking ? <p role="status">Checking local agents…</p> : <p>{checkError || "No agents configured yet."} <button type="button" className="text-button" onClick={onSetup}>Agent setup</button></p>}
      {!!names.length && !selectedWorkers.length && <p>Select at least one worker.</p>}
      {report && !report.git && <p role="alert">Git is required. <button type="button" onClick={onSetup}>Agent setup</button></p>}
      <div className="brief-footer">
        <div className="brief-settings"><button type="button" onClick={() => setModal("advanced")}>Advanced settings</button>
          <small>{delivery === "auto" ? "Auto-land" : delivery === "review" ? "Review before landing" : "Auto-land and deploy"}</small>
        </div>
        <button className="primary" type="submit" disabled={!ready}>Start run</button>
      </div>
    </form>
    {modal === "advanced" && <Dialog title="Advanced settings" onClose={() => setModal("")}>
      <div className="options brief-options">
        <label>Delivery<select value={delivery} onChange={(event) => {
          setDelivery(event.target.value);
          if (event.target.value === "deploy" && destinations === null && !loadingDestinations) loadDestinations();
        }}><option value="auto">Auto-land</option><option value="review">Review before landing</option><option value="deploy">Auto-land and deploy</option></select></label>
        <label>Worker routing<select value={routing} disabled={selectedWorkers.length < 2} onChange={(event) => setRouting(event.target.value)}>
          <option value="manual">Coordinator chooses</option><option value="balanced">Balance benchmark score and cost</option>
          <option value="quality">Highest benchmark score</option><option value="economy">Favor lower cost</option>
        </select></label>
        {delivery === "deploy" && <div className="destination-field">
          <label>Destination<select value={destination} disabled={loadingDestinations} onChange={(event) => setDestination(event.target.value)}>
            <option value="">Choose a destination</option>{destinations?.map((name) => <option key={name}>{name}</option>)}</select></label>
          {loadingDestinations ? <p role="status">Loading destinations…</p> : destinationError || !destinations?.length ?
            <><p role={destinationError ? "alert" : undefined}>{destinationError || <>Add a destination to <code>destinations.json</code>.</>}</p>
              <button type="button" onClick={loadDestinations}>Reload destinations</button></> :
            <p>The tested commit can land and deploy to this destination automatically.</p>}
        </div>}
        <label>Advisor<select aria-label="Advisor" value={names.includes(advisor) ? advisor : ""} onChange={(event) => setAdvisor(event.target.value)}><option value="">Use coordinator</option>{names.map((name) => <option key={name} value={name}>{label(name)}</option>)}</select></label>
        <label>Token target<input type="number" min="0" step="1" value={tokens} onChange={(event) => setTokens(event.target.value)} placeholder="Optional" /></label>
        <label>Target branch<input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="Current branch" /></label>
        <div className="file-field"><span>Initial import files</span><button type="button" disabled={!path.trim()} onClick={() => setPickFiles(true)}>
          {initialFiles.length ? `Choose files (${initialFiles.length})` : "Choose files…"}
        </button></div>
        <label className="checkbox"><input type="checkbox" checked={compare} onChange={(event) => setCompare(event.target.checked)} />Compare approaches against a baseline</label>
      </div>
      <div className="dialog-footer"><button className="primary" onClick={() => setModal("")}>Done</button></div>
    </Dialog>}
    {pickFiles && <ImportPicker {...{ api, path }} selected={initialFiles} onChange={setInitialFiles} onClose={() => setPickFiles(false)} />}
    {modal === "model" && profile && <ModelSettings name={selectedCoordinator} profile={profile} save={saveModel} onClose={() => setModal("")} />}
    {modal === "original" && <Dialog title="Original brief" onClose={() => setModal("")}>
      <p className="original-brief">{original}</p><button onClick={() => { setPrompt(original); setOriginal(""); setModal(""); }}>Restore original</button>
    </Dialog>}
  </section>;
}
