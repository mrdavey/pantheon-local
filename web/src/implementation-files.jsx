import React, { useEffect, useState } from "react";
import { File, Folder, ChevronRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { Loading } from "./status";
import { Markdown } from "./markdown";
import { Dialog } from "./dialog";
import { changeRoute } from "./navigation";

export function ImplementationFiles({ api, run, candidate, path = "", onClose, explain }) {
  const [result, setResult] = useState(null);
  const [retry, setRetry] = useState(0);
  const endpoint = "/runs/" + run.id + "/implementations/" + candidate.id + "/files?" + new URLSearchParams({ path });
  useEffect(() => {
    const request = new AbortController();
    let imageUrl;
    api(endpoint, undefined, request.signal).then(data => {
      if (request.signal.aborted) return;
      if (data.kind === "image") {
        const bytes = Uint8Array.from(atob(data.bytes), ch => ch.charCodeAt(0));
        imageUrl = URL.createObjectURL(new Blob([bytes], {type:data.mime}));
      }
      setResult({ endpoint, data, imageUrl });
    }).catch(error => {
      if (!request.signal.aborted) setResult({ endpoint, error: error.message });
    });
    return () => { request.abort(); if (imageUrl) URL.revokeObjectURL(imageUrl); };
  }, [api, endpoint, retry]);
  const open = file => changeRoute({ file });
  const parts = path.split("/").filter(Boolean);
  const current = result?.endpoint === endpoint ? result : null;
  const data = current?.data;
  const entries = data?.entries?.slice().sort((a,b) =>
    Number(b.kind === "tree") - Number(a.kind === "tree") || a.name.localeCompare(b.name));
  return <Dialog title="Implementation files" className="snapshot-dialog" onClose={onClose} onExplain={() => explain("implementation",candidate.id)}>
    <nav aria-label="File path" className="snapshot-path">
      <Button variant="link" onClick={() => open(null)}>Project</Button>
      {parts.map((part,i) => <React.Fragment key={i}><ChevronRight aria-hidden="true"/><Button variant="link" onClick={() => open(parts.slice(0,i+1).join("/"))}>{part}</Button></React.Fragment>)}
    </nav>
    {!current && <Loading>Loading files…</Loading>}
    {current?.error && <div role="alert">{current.error} <Button variant="outline" onClick={() => {setResult(null);setRetry(n => n+1);}}>Try again</Button></div>}
    {entries && <ul className="snapshot-list">{entries.map(entry => <li key={entry.name}><Button variant="ghost" disabled={entry.kind === "commit"} onClick={() => open([...parts,entry.name].join("/"))}>
      {entry.kind === "tree" ? <Folder/> : <File/>}{entry.name}{entry.kind === "commit" && " (submodule)"}</Button></li>)}</ul>}
    {entries?.length === 0 && <p>Empty directory.</p>}
    {data?.kind === "text" && (data.text === "" ? <p>Empty file.</p> : /\.(md|markdown|mdown|mkd)$/i.test(path) ? <Markdown>{data.text}</Markdown> : <pre className="snapshot-source">{data.text}</pre>)}
    {current?.imageUrl && <img src={current.imageUrl} alt={parts.at(-1)} className="snapshot-image"/>}
    {data?.reason && <p>{data.reason}</p>}
  </Dialog>;
}
