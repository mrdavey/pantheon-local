import React, { useEffect, useId, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { Input } from "./components/ui/input";
import { Button } from "./components/ui/button";
import { Label } from "./components/ui/label";
import { Loading } from "./status";

export function ModelPicker({ api, name, value, disabled, onChange }) {
  const id = useId(), input = useRef(null);
  const [open, setOpen] = useState(false);
  const [models, setModels] = useState(null);
  const [failed, setFailed] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(-1);
  useEffect(() => {
    const request = new AbortController();
    api("/setup/models?profile=" + encodeURIComponent(name), undefined, request.signal)
      .then(list => { if (!request.signal.aborted) setModels(list); })
      .catch(() => { if (!request.signal.aborted) setFailed(true); });
    return () => request.abort();
  }, [api, name]);
  const options = (models || []).filter(model => model.toLowerCase().includes(query.toLowerCase()));
  const choose = model => {
    onChange(model);
    input.current.focus();
    setOpen(false);
    setActive(-1);
  };
  const keys = event => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setOpen(true);
      if (options.length) {
        const index = event.key === "ArrowDown" ? Math.min(active + 1, options.length - 1) : active < 0 ? options.length - 1 : Math.max(0, active - 1);
        setActive(index);
        document.getElementById(id + "-" + index)?.scrollIntoView({block:"nearest"});
      }
    } else if (event.key === "Enter" && open) {
      event.preventDefault();
      if (options[active]) choose(options[active]);
      else setOpen(false);
    } else if (event.key === "Escape" && open) {
      event.preventDefault();
      event.stopPropagation();
      setOpen(false);
    }
  };
  return <div className="model-picker" onBlur={event => {
    if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
  }}>
    <Label htmlFor={id}>Model ID</Label>
    <div className="model-picker-field">
      <Input id={id} ref={input} role="combobox" aria-expanded={open} aria-autocomplete="list"
        aria-controls={id + "-list"} aria-activedescendant={open && options[active] ? id + "-" + active : undefined}
        value={value} disabled={disabled} autoComplete="off" maxLength={150} placeholder="Use the CLI default"
        onFocus={() => { setQuery(""); setActive(-1); setOpen(true); }}
        onChange={event => { onChange(event.target.value); setQuery(event.target.value); setActive(-1); setOpen(true); }} onKeyDown={keys} />
      <Button type="button" variant="ghost" size="icon" disabled={disabled} aria-label="Show model IDs" aria-expanded={open}
        onMouseDown={event => event.preventDefault()} onClick={() => {
          input.current.focus(); setQuery(""); setActive(-1); setOpen(!open);
        }}><ChevronDown /></Button>
    </div>
    <div id={id + "-list"} role="listbox" aria-label="Model suggestions" hidden={!open}>
      {open && options.map((model,index) => <Button type="button" role="option" variant="ghost"
        key={model} id={id + "-" + index} tabIndex={-1} aria-selected={active === index}
        onMouseDown={event => event.preventDefault()} onClick={() => choose(model)}>{model}</Button>)}
    </div>
    {open && !options.length && <small role="status">{failed ? "Model list unavailable. Enter an ID." : !models ? <Loading>Loading models…</Loading> : "No matching models. You can enter an ID."}</small>}
  </div>;
}
