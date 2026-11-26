import { Card } from "./components/ui/card";
import { Loading } from "./status.jsx";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Textarea } from "./components/ui/textarea";
import { NativeSelect } from "./components/ui/native-select";
import { NativeSelectOption } from "./components/ui/native-select";
import { Label } from "./components/ui/label";
import { Checkbox } from "./components/ui/checkbox";
import React, { useEffect, useState } from "react";
import { Dialog } from "./dialog.jsx";
import { availableNames, ModelSettings, ModelSettingsForm, profileLabel, runModel } from "./setup.jsx";

import { addAttachments, AttachmentButton, encodeAttachments, FileList } from "./brief-files.jsx";
import "./brief.css";

import { changeRoute, useRoute } from "./navigation";

import { runProjectPath } from "./run-summary";
import { branchName } from "./review";

function defaultWorkers(profiles) {
  if (!profiles.length) return [];
  return Array.from({length:3}, (_, key) => ({key, profile:profiles[key % profiles.length]}));
}

function AddAgent({api, names, profiles, onAdd, onClose}) {
  const [name, setName] = useState(names[0]);
  const selected = names.includes(name) ? name : names[0];
  return <Dialog title="Add agent" onClose={onClose}>
    <Label>Harness<NativeSelect value={selected} onChange={event => setName(event.target.value)}>
      {names.map(id => <NativeSelectOption key={id} value={id}>{profileLabel(id, profiles[id])}</NativeSelectOption>)}
    </NativeSelect></Label>
    {selected && <ModelSettingsForm api={api} key={selected} name={selected} profile={profiles[selected]} agentOnly
      submitLabel="Add to pool" onClose={onClose} save={({model,effort}) => onAdd({profile:selected,model,effort})} />}
  </Dialog>;
}

export function Brief({ setup, report, checking, checkError, cwd, followup, api, agentChoice, onSetup, onCreate, perform, busy }) {
  const [path, setPath] = useState(() => followup ? runProjectPath(followup) : cwd);
  const [prompt, setPrompt] = useState("");
  const [suggestion, setSuggestion] = useState("");
  const [original, setOriginal] = useState("");
  const [coordinator, setCoordinator] = useState(agentChoice?.name || setup.coordinator || "");
  const [coordinatorModels, setCoordinatorModels] = useState({});
  const [workers, setWorkers] = useState(null);
  const [advisor, setAdvisor] = useState("");
  const route = useRoute();
  const modelName = route.profile;
  const [allowChildren, setAllowChildren] = useState(true);
  const [unlimited, setUnlimited] = useState(true);
  const [limit, setLimit] = useState("2");
  const validLimit = !allowChildren || unlimited || (/^[1-9][0-9]*$/.test(limit) && Number(limit) <= 4294967295);
  const [tokens, setTokens] = useState("");
  const [routing, setRouting] = useState("manual");
  const [initialFiles, setInitialFiles] = useState([]);
  const [target, setTarget] = useState(() => branchName(followup?.repo?.Target) || "");
  const [compare, setCompare] = useState(false);
  const [delivery, setDelivery] = useState("auto");
  const [destination, setDestination] = useState("");
  const [destinations, setDestinations] = useState(null);
  const [destinationError, setDestinationError] = useState("");
  const [loadingDestinations, setLoadingDestinations] = useState(false);
  const panel = route.page === "new" ? route.panel : "";
  const setModal = (panel,profile,slot) => changeRoute({panel,profile,slot});
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
      setWorkers(defaultWorkers([agentChoice.name]));
    }
  }, [agentChoice]);
  const names = availableNames(report);
  const selectedCoordinator = names.includes(coordinator) ? coordinator : names[0] || "";
  const profile = {...setup.profiles?.[selectedCoordinator],...coordinatorModels[selectedCoordinator]};
  const preferred = (setup.workers || []).filter((name) => names.includes(name));
  const suggestions = agentChoice && names.includes(agentChoice.name) ? [agentChoice.name] : preferred.length ? preferred : names;
  const selectedWorkers = (workers ?? defaultWorkers(suggestions)).filter(slot => names.includes(slot.profile));
  const agentCount = selectedWorkers.length + 1;
  function updateWorker(key, values) {
    setWorkers(selectedWorkers.map(slot => slot.key === key ? {...slot,...values} : slot));
  }
  const editingSlot = selectedWorkers.find(slot => String(slot.key) === route.slot && slot.profile === modelName);
  const editingCoordinator = route.slot == null && modelName === selectedCoordinator;
  const modal = panel === "model" && !editingSlot && !editingCoordinator ? "team" : panel;
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
  const ready = validLimit && !formBusy && report?.git && path.trim() && prompt.trim() && selectedCoordinator && selectedWorkers.length && (delivery !== "deploy" || destination);
  const create = (event) => {
    event.preventDefault();
    if (!ready) return;
    perform(async () => {
      const run = await api("/runs", {
        attachments: await encodeAttachments(files), path, prompt, original_prompt: original, coordinator: selectedCoordinator, coordinator_settings: coordinatorModels[selectedCoordinator], worker_pool: selectedWorkers.map(({key,...worker}) => worker),
        routing: selectedWorkers.length > 1 ? routing : "manual", token_target: Number(tokens),
        subagents: allowChildren, subagent_limit: allowChildren && !unlimited ? Number(limit) : null,
        initial_files: initialFiles, target, ...(followup && target ? {base_from_target:true} : {}), compare, delivery, destination: delivery === "deploy" ? destination : "", recipe: { checks: [] },
      });
      onCreate(run);
    }, "Starting run…");
  };
  return <section className="panel brief">
    <h1 tabIndex={-1}>Start a run</h1>
    <form className="new-run-form" onSubmit={create}>
      <Card className={"goal-composer" + (dragging ? " dragging" : "")}
        onDragOver={event => { if (event.dataTransfer.types.includes("Files")) { event.preventDefault(); setDragging(true); } }}
        onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget)) setDragging(false); }}
        onDrop={event => { event.preventDefault(); setDragging(false); addFiles(Array.from(event.dataTransfer.files)); }}>
        <Label htmlFor="run-goal">What do you want to build?</Label>
        <Textarea id="run-goal" required rows={4} value={prompt} placeholder="Describe the change you want to make…" onChange={event => setPrompt(event.target.value)} />
        <FileList names={files.map(file => file.name)} remove={i => setFiles(current => current.filter((_, index) => i !== index))} />
        {fileError && <p role="alert" className="notice error">{fileError}</p>}
        {briefError && <p role="alert" className="notice error">{briefError}</p>}
        {briefNotice && <Loading>{briefNotice}</Loading>}
        <div className="composer-tools">
          <AttachmentButton add={addFiles} busy={formBusy} />
          <Button variant="outline" type="button" className="text-button" disabled={formBusy || !prompt.trim() || !advisorName}
            onClick={() => runBriefTask(async () => setSuggestion(await api("/advice", { profile: advisorName, prompt, attachments: await encodeAttachments(files) })), "Improving brief…")}>Improve brief</Button>
          <Button variant="default" className="primary start-run" aria-label="Start run" type="submit" disabled={!ready}>Start run <span aria-hidden="true">↑</span></Button>
        </div>
      </Card>
      {original && <Button variant="outline" type="button" className="text-button" onClick={() => setModal("original")}>Original brief</Button>}
      {suggestion && <div className="answer">
        <Label>Suggested brief<Textarea rows={4} value={suggestion} onChange={(event) => setSuggestion(event.target.value)} /></Label>
        <Button variant="outline" type="button" onClick={() => { setOriginal(original || prompt); setPrompt(suggestion); setSuggestion(""); }}>Use suggestion</Button>
        <Button variant="outline" type="button" onClick={() => setSuggestion("")}>Dismiss</Button>
      </div>}
      <div className="project-picker">
        <Label htmlFor="project-folder">Project folder</Label>
        <div className="path-controls">
          <Input id="project-folder" required value={path} spellCheck={false} autoCapitalize="none" onChange={(event) => changePath(event.target.value)} placeholder="/path/to/project" />
          <Button variant="outline" type="button" disabled={formBusy} onClick={() => runBriefTask(async () => {
            const folder = await api("/project-folder", {});
            if (folder) changePath(folder);
          }, "Choosing a folder…")}>Browse…</Button>
        </div>
      </div>
      {followup && <Label>Target branch<Input value={target} onChange={event => setTarget(event.target.value)} placeholder="Current branch" /></Label>}
      <div className="run-preferences">
        {names.length ? <Button variant="outline" type="button" className="team-control" onClick={() => setModal("team")} aria-label="Team">
          <strong>Team</strong><span>{agentCount} {agentCount === 1 ? "agent" : "agents"} · {label(selectedCoordinator)} coordinates</span>
        </Button> : checking ? <Loading>Checking local agents…</Loading> : <p>{checkError || "No agents configured yet."} <Button variant="outline" type="button" onClick={onSetup}>Agent setup</Button></p>}
        <Button variant="outline" type="button" className="team-control" onClick={() => setModal("advanced")} aria-label="Advanced settings">
          <strong>Advanced settings</strong><span>{delivery === "auto" ? "Auto-land" : delivery === "review" ? "Review before landing" : "Auto-land and deploy"}</span>
        </Button>
      </div>
      {!!names.length && !selectedWorkers.length && <p>Select at least one worker.</p>}
      {report && !report.git && <p role="alert">Git is required. <Button variant="outline" type="button" onClick={onSetup}>Agent setup</Button></p>}
    </form>
    {(["team","model","add-agent"].includes(modal)) && <Dialog title="Team" onClose={() => { if (modal === "team") setModal(""); }}>
      <div className="roles">
        <div><Label>Coordinator<NativeSelect aria-label="Coordinator" required title={label(selectedCoordinator)} value={selectedCoordinator} onChange={(event) => setCoordinator(event.target.value)}>
          {names.map((name) => <NativeSelectOption key={name} value={name}>{label(name)}</NativeSelectOption>)}</NativeSelect></Label>
          <Button variant="outline" type="button" className="model-link text-button" onClick={() => { setModal("model",selectedCoordinator); }}>
            {[profile?.model || profile?.detected?.label || "Agent default", profile?.effort || profile?.detected?.effort].filter(Boolean).join(" · ")} · Model settings
          </Button>
        </div>
        <fieldset className="worker-pool"><legend>Worker pool</legend>
          <Button type="button" className="add-worker" onClick={() => setModal("add-agent")}>Add agent</Button>
          <ol tabIndex={0} aria-label="Workers in this run">{selectedWorkers.map((slot,index) => {
            const {model,effort} = runModel({...setup.profiles[slot.profile],...slot});
            return <li key={slot.key}><div className="worker-pool-heading"><div><strong>{index+1}. {label(slot.profile)}</strong>
              <small>{model} · {effort}</small></div><div className="actions">
              <Button variant="outline" aria-label={`Settings for worker ${index+1}`} onClick={() => setModal("model",slot.profile,String(slot.key))}>Settings</Button>
              <Button variant="ghost" aria-label={`Remove worker ${index+1}`} onClick={() => setWorkers(selectedWorkers.filter(w => w.key !== slot.key))}>Remove</Button></div></div>
            </li>;
          })}</ol>
          {!selectedWorkers.length && <p>Add a worker to start this run.</p>}
        </fieldset>
      </div>
      <fieldset className="subagent-settings"><legend>Delegation</legend>
        <Label className="checkbox"><Checkbox checked={allowChildren} onCheckedChange={setAllowChildren} />Allow subagents</Label>
        {allowChildren && <><Label className="checkbox"><Checkbox checked={unlimited} onCheckedChange={setUnlimited} />Unlimited subagents</Label>
          {!unlimited && <Label>Concurrent subagents<Input type="number" min="1" max="4294967295" value={limit} aria-invalid={!validLimit} onChange={e => setLimit(e.target.value)} /></Label>}
          {!validLimit && <p role="alert">Enter a positive whole number.</p>}</>}
      </fieldset>
      <div className="dialog-footer"><Button variant="outline" type="button" disabled={!validLimit || !selectedWorkers.length} onClick={() => setModal("")}>Done</Button></div>
      {modal === "add-agent" && <AddAgent api={api} names={names} profiles={setup.profiles} onClose={() => setModal("team")}
        onAdd={slot => setWorkers([...selectedWorkers,{...slot,key:crypto.randomUUID()}])}/>}
      {modal === "model" && <ModelSettings api={api} key={editingSlot?.key ?? "coordinator"} name={modelName}
        runLabel={editingSlot ? `worker ${selectedWorkers.indexOf(editingSlot)+1} · ${label(modelName)}` : `coordinator · ${label(modelName)}`}
        profile={editingSlot ? {...setup.profiles[modelName],...editingSlot} : profile}
        save={({model,effort}) => editingSlot ? updateWorker(editingSlot.key,{model,effort}) :
          setCoordinatorModels(current => ({...current,[modelName]:{model,effort}}))}
        onClose={() => setModal("team")} />}
    </Dialog>}
    {modal === "advanced" && <Dialog className="advanced-sheet" title="Advanced settings" onClose={() => { if (!picking) setModal(""); }}>
      <div className="options brief-options">
        <Label>Delivery<NativeSelect value={delivery} onChange={(event) => {
          setDelivery(event.target.value);
          if (event.target.value === "deploy" && destinations === null && !loadingDestinations) loadDestinations();
        }}><NativeSelectOption value="auto">Auto-land</NativeSelectOption><NativeSelectOption value="review">Review before landing</NativeSelectOption><NativeSelectOption value="deploy">Auto-land and deploy</NativeSelectOption></NativeSelect></Label>
        <Label>Worker routing<NativeSelect value={routing} disabled={selectedWorkers.length < 2} onChange={(event) => setRouting(event.target.value)}>
          <NativeSelectOption value="manual">Coordinator chooses</NativeSelectOption><NativeSelectOption value="balanced">Balance benchmark score and cost</NativeSelectOption>
          <NativeSelectOption value="quality">Highest benchmark score</NativeSelectOption><NativeSelectOption value="economy">Favor lower cost</NativeSelectOption>
        </NativeSelect></Label>
        {delivery === "deploy" && <div className="destination-field">
          <Label>Destination<NativeSelect value={destination} disabled={loadingDestinations} onChange={(event) => setDestination(event.target.value)}>
            <NativeSelectOption value="">Choose a destination</NativeSelectOption>{destinations?.map((name) => <NativeSelectOption key={name}>{name}</NativeSelectOption>)}</NativeSelect></Label>
          {loadingDestinations ? <Loading>Loading destinations…</Loading> : destinationError || !destinations?.length ?
            <><p role={destinationError ? "alert" : undefined}>{destinationError || <>Add a destination to <code>destinations.json</code>.</>}</p>
              <Button variant="outline" type="button" onClick={loadDestinations}>Reload destinations</Button></> :
            <p>The tested commit can land and deploy to this destination automatically.</p>}
        </div>}
        <Label>Advisor<NativeSelect aria-label="Advisor" value={names.includes(advisor) ? advisor : ""} onChange={(event) => setAdvisor(event.target.value)}><NativeSelectOption value="">Use coordinator</NativeSelectOption>{names.map((name) => <NativeSelectOption key={name} value={name}>{label(name)}</NativeSelectOption>)}</NativeSelect></Label>
        <Label>Token target<Input type="number" min="0" step="1" value={tokens} onChange={(event) => setTokens(event.target.value)} placeholder="Optional" /></Label>
        {!followup && <Label>Target branch<Input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="Current branch" /></Label>}
        <div className="file-field"><span>Initial import files</span>
          <Button variant="outline" type="button" disabled={!path.trim() || picking} onClick={pickImports}>{picking ? "Choosing files…" : initialFiles.length ? "Add more files…" : "Choose files…"}</Button>
          <FileList names={initialFiles} remove={i => setInitialFiles(current => current.filter((_, index) => i !== index))} />
          {importError && <p role="alert" className="notice error">{importError}</p>}
        </div>
        <Label className="checkbox"><Checkbox checked={compare} onCheckedChange={setCompare} />Compare approaches against a baseline</Label>
      </div>
      <div className="dialog-footer"><Button variant="default" className="primary" disabled={picking} onClick={() => setModal("")}>Done</Button></div>
    </Dialog>}
    {modal === "original" && original && <Dialog title="Original brief" onClose={() => setModal("")}>
      <p className="original-brief">{original}</p><Button variant="outline" onClick={() => { setPrompt(original); setOriginal(""); setModal(""); }}>Restore original</Button>
    </Dialog>}
  </section>;
}
