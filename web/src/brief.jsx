import React, { useEffect, useState } from "react";
import { Dialog } from "./dialog.jsx";
import { availableNames, ModelSettings, profileLabel } from "./setup.jsx";

import { addAttachments, AttachmentButton, encodeAttachments, FileList } from "./brief-files.jsx";
import "./brief.css";

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
  const [files, setFiles] = useState([]);
  const [fileError, setFileError] = useState("");
  const [importError, setImportError] = useState("");
  const [briefNotice, setBriefNotice] = useState("");
  const [briefError, setBriefError] = useState("");
  const [picking, setPicking] = useState(false);
  const [dragging, setDragging] = useState(false);
  async function runBriefTask(fn, message) {
    if (briefNotice) return;
    setBriefError("");
    setBriefNotice(message);
    try { await fn(); }
    catch (error) { setBriefError(error.message); }
    finally { setBriefNotice(""); }
  }
  function addFiles(additions) {
    try { setFiles(addAttachments(files, additions)); setFileError(""); }
    catch (error) { setFileError(error.message); }
  }
  async function pickImports() {
    setImportError("");
    setPicking(true);
    try {
      const selection = await api("/pick-initial-files", { path });
      setInitialFiles(current => [...new Set([...current, ...selection])]);
    } catch (error) { setImportError(error.message); }
    finally { setPicking(false); }
  }
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
  const changePath = (next) => { if (next !== path) { setPath(next); setInitialFiles([]); setImportError(""); } };
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
  const formBusy = busy || Boolean(briefNotice);
  const ready = !formBusy && report?.git && path.trim() && prompt.trim() && selectedCoordinator && selectedWorkers.length && (delivery !== "deploy" || destination);
  const create = (event) => {
    event.preventDefault();
    if (!ready) return;
    perform(async () => {
      const run = await api("/runs", {
        attachments: await encodeAttachments(files), path, prompt, original_prompt: original, coordinator: selectedCoordinator, workers: selectedWorkers,
        routing: selectedWorkers.length > 1 ? routing : "manual", token_target: Number(tokens),
        initial_files: initialFiles, target, compare, delivery, destination: delivery === "deploy" ? destination : "", recipe: { checks: [] },
      });
      onCreate(run);
    }, "Starting run…");
  };
  return <section className="panel brief">
    <h1 tabIndex={-1}>Start a run</h1>
    <form className="new-run-form" onSubmit={create}>
      <div className={"goal-composer" + (dragging ? " dragging" : "")}
        onDragOver={event => { if (event.dataTransfer.types.includes("Files")) { event.preventDefault(); setDragging(true); } }}
        onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget)) setDragging(false); }}
        onDrop={event => { event.preventDefault(); setDragging(false); addFiles(Array.from(event.dataTransfer.files)); }}>
        <label htmlFor="run-goal">What do you want to build?</label>
        <textarea id="run-goal" required rows={4} value={prompt} placeholder="Describe the change you want to make…" onChange={event => setPrompt(event.target.value)} />
        <FileList names={files.map(file => file.name)} remove={i => setFiles(current => current.filter((_, index) => i !== index))} />
        {fileError && <p role="alert" className="notice error">{fileError}</p>}
        {briefError && <p role="alert" className="notice error">{briefError}</p>}
        {briefNotice && <p role="status" className="notice">{briefNotice}</p>}
        <div className="composer-tools">
          <AttachmentButton add={addFiles} busy={formBusy} />
          <button type="button" className="text-button" disabled={formBusy || !prompt.trim() || !advisorName}
            onClick={() => runBriefTask(async () => setSuggestion(await api("/advice", { profile: advisorName, prompt, attachments: await encodeAttachments(files) })), "Improving brief…")}>Improve brief</button>
          <button className="primary start-run" aria-label="Start run" type="submit" disabled={!ready}>Start run <span aria-hidden="true">↑</span></button>
        </div>
      </div>
      {original && <button type="button" className="text-button" onClick={() => setModal("original")}>Original brief</button>}
      {suggestion && <div className="answer">
        <label>Suggested brief<textarea rows={4} value={suggestion} onChange={(event) => setSuggestion(event.target.value)} /></label>
        <button type="button" onClick={() => { setOriginal(original || prompt); setPrompt(suggestion); setSuggestion(""); }}>Use suggestion</button>
        <button type="button" onClick={() => setSuggestion("")}>Dismiss</button>
      </div>}
      <div className="project-picker">
        <label htmlFor="project-folder">Project folder</label>
        <div className="path-controls">
          <input id="project-folder" required value={path} spellCheck={false} autoCapitalize="none" onChange={(event) => changePath(event.target.value)} placeholder="/path/to/project" />
          <button type="button" disabled={formBusy} onClick={() => runBriefTask(async () => {
            const folder = await api("/project-folder", {});
            if (folder) changePath(folder);
          }, "Choosing a folder…")}>Browse…</button>
        </div>
      </div>
      <div className="run-preferences">
        {names.length ? <button type="button" className="team-control" onClick={() => setModal("team")} aria-label="Team">
          <strong>Team</strong><span>{label(selectedCoordinator)} · {selectedWorkers.length} {selectedWorkers.length === 1 ? "worker" : "workers"}</span>
        </button> : checking ? <p role="status">Checking local agents…</p> : <p>{checkError || "No agents configured yet."} <button type="button" onClick={onSetup}>Agent setup</button></p>}
        <button type="button" className="team-control" onClick={() => setModal("advanced")} aria-label="Advanced settings">
          <strong>Advanced settings</strong><span>{delivery === "auto" ? "Auto-land" : delivery === "review" ? "Review before landing" : "Auto-land and deploy"}</span>
        </button>
      </div>
      {!!names.length && !selectedWorkers.length && <p>Select at least one worker.</p>}
      {report && !report.git && <p role="alert">Git is required. <button type="button" onClick={onSetup}>Agent setup</button></p>}
    </form>
    {(modal === "team" || modal === "model") && <Dialog title="Team" onClose={() => setModal("")}>
      <div className="roles">
        <div><label>Coordinator<select aria-label="Coordinator" required title={label(selectedCoordinator)} value={selectedCoordinator} onChange={(event) => setCoordinator(event.target.value)}>
          {names.map((name) => <option key={name} value={name}>{label(name)}</option>)}</select></label>
          <button type="button" className="model-link text-button" onClick={() => setModal("model")}>
            {[profile?.detected?.label || profile?.model || "Agent default", profile?.detected?.effort].filter(Boolean).join(" · ")} · Model settings
          </button>
        </div>
        <fieldset><legend>Workers</legend>{names.map((name) => <label className="checkbox" key={name}><input type="checkbox" checked={selectedWorkers.includes(name)}
          onChange={(event) => setWorkers(event.target.checked ? [...selectedWorkers, name] : selectedWorkers.filter((worker) => worker !== name))} />{label(name)}</label>)}</fieldset>
      </div>
      <div className="dialog-footer"><button type="button" onClick={() => setModal("")}>Done</button></div>
    </Dialog>}
    {modal === "advanced" && <Dialog className="advanced-sheet" title="Advanced settings" onClose={() => { if (!picking) setModal(""); }}>
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
        <div className="file-field"><span>Initial import files</span>
          <button type="button" disabled={!path.trim() || picking} onClick={pickImports}>{picking ? "Choosing files…" : initialFiles.length ? "Add more files…" : "Choose files…"}</button>
          <FileList names={initialFiles} remove={i => setInitialFiles(current => current.filter((_, index) => i !== index))} />
          {importError && <p role="alert" className="notice error">{importError}</p>}
        </div>
        <label className="checkbox"><input type="checkbox" checked={compare} onChange={(event) => setCompare(event.target.checked)} />Compare approaches against a baseline</label>
      </div>
      <div className="dialog-footer"><button className="primary" disabled={picking} onClick={() => setModal("")}>Done</button></div>
    </Dialog>}
    {modal === "model" && profile && <ModelSettings name={selectedCoordinator} profile={profile} save={saveModel} onClose={() => setModal("team")} />}
    {modal === "original" && <Dialog title="Original brief" onClose={() => setModal("")}>
      <p className="original-brief">{original}</p><button onClick={() => { setPrompt(original); setOriginal(""); setModal(""); }}>Restore original</button>
    </Dialog>}
  </section>;
}
