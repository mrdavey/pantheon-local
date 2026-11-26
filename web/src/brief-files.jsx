import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { Plus, Paperclip } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "./components/ui/popover";
import React, { useRef, useState } from "react";

const limit = 16 * 1024 * 1024;
export function addAttachments(files, additions) {
  const next = [...files];
  for (const file of additions) {
    if (!next.includes(file)) next.push(file);
  }
  if (next.length > 16 || next.reduce((total, file) => total + file.size, 0) > limit) {
    throw new Error("Attach up to 16 files, totaling 16 MiB or less.");
  }
  return next;
}

export async function encodeAttachments(files) {
  return Promise.all(files.map(file => new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Couldn’t read " + file.name));
    reader.onload = () => resolve({ name: file.name, data: reader.result.slice(reader.result.indexOf(",") + 1) });
    reader.readAsDataURL(file);
  })));
}

export function AttachmentButton({ add, busy, menu = false, label = "Brief attachments" }) {
  const picker = useRef(null);
  const [open, setOpen] = useState(false);
  return <>
    <Input ref={picker} type="file" multiple hidden aria-label={label} onChange={event => {
      add(Array.from(event.target.files));
      event.target.value = "";
    }} />
    {menu ? <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild><Button variant="ghost" size="icon" type="button" disabled={busy} aria-label="Attachment options" title="Attachment options"><Plus/></Button></PopoverTrigger>
      <PopoverContent side="top" align="start" aria-label="Attachments"><Button variant="ghost" type="button" onClick={() => {picker.current.click(); setOpen(false);}}><Paperclip/>Attach files</Button></PopoverContent>
    </Popover> : <Button variant="outline" type="button" className="attach-button" disabled={busy} onClick={() => picker.current.click()}><Paperclip/>Attach files</Button>}
  </>;
}

export function FileList({ names, remove, disabled }) {
  if (!names.length) return null;
  return <ul className="brief-files">{names.map((name, index) =>
    <li key={index}><span title={name}>{name}</span><Button variant="outline" type="button" disabled={disabled} aria-label={"Remove " + name} onClick={() => remove(index)}>×</Button></li>)}</ul>;
}
