import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import React, { useRef } from "react";

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

export function AttachmentButton({ add, busy, label = "Brief attachments" }) {
  const picker = useRef(null);
  return <>
    <Input ref={picker} type="file" multiple hidden aria-label={label} onChange={event => {
      add(Array.from(event.target.files));
      event.target.value = "";
    }} />
    <Button variant="outline" type="button" className="attach-button" disabled={busy} onClick={() => picker.current.click()}>
      <svg aria-hidden="true" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round">
        <path d="m8 13 7.7-7.7a3.2 3.2 0 0 1 4.5 4.5L10 20a5 5 0 0 1-7-7L13.7 2.3M6.5 15.5l9-9"/>
      </svg> Attach files
    </Button>
  </>;
}

export function FileList({ names, remove, disabled }) {
  if (!names.length) return null;
  return <ul className="brief-files">{names.map((name, index) =>
    <li key={index}><span title={name}>{name}</span><Button variant="outline" type="button" disabled={disabled} aria-label={"Remove " + name} onClick={() => remove(index)}>×</Button></li>)}</ul>;
}
