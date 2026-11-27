import { Button } from "./components/ui/button";
import { NativeSelect } from "./components/ui/native-select";
import { NativeSelectOption } from "./components/ui/native-select";
import { Label } from "./components/ui/label";
import React, { useMemo, useState } from "react";
import { Markdown } from "./markdown.jsx";

function parsed(text) {
  try { return JSON.parse(text); } catch { return undefined; }
}

function label(key) {
  const words = String(key).replace(/([a-z])([A-Z])/g, "$1 $2").replace(/[._/-]+/g, " ").trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function Command({ argv }) {
  // Keep argument boundaries visible, including empty and multiline arguments.
  const quote = (arg) => /^[a-zA-Z0-9_@%+=:,./-]+$/.test(arg) ? arg : "'" + arg.replaceAll("'", "'\\''") + "'";
  return <code className="shell-command">{Array.isArray(argv) ? argv.map((arg) => quote(String(arg))).join(" ") : String(argv ?? "")}</code>;
}

export function Fields({ value, depth = 0 }) {
  if (value === null || value === undefined) return <span>—</span>;
  if (typeof value !== "object") return <span>{String(value)}</span>;
  if (depth > 6) return <pre>{JSON.stringify(value, null, 2)}</pre>;
  if (Array.isArray(value)) return <ul className="event-list">{value.map((entry, i) =>
    <li key={i}><Fields value={entry} depth={depth + 1} /></li>)}</ul>;
  return <dl className="event-fields">{Object.entries(value).map(([key, entry]) =>
    <div key={key}><dt>{label(key)}</dt><dd><Fields value={entry} depth={depth + 1} /></dd></div>)}</dl>;
}

function entry(title, body, kind = "message") { return { title, body, kind }; }

function claudeBlock(block) {
  if (!block || typeof block !== "object") return entry("Message", String(block || ""));
  if (block.type === "tool_use") {
    if (block.input?.command) return entry(block.name || "Command", block.input.command, "command");
    return entry(block.name || "Tool call", block.input, "fields");
  }
  if (block.type === "tool_result") {
    if (typeof block.content === "string") return entry("Tool output", block.content, "terminal");
    return entry("Tool output", block.content, "fields");
  }
  return entry(label(block.type || "Message"), block.text ?? block.thinking ?? block, block.text || block.thinking ? "message" : "fields");
}

function agentEvents(event, agentOutput = false) {
  const { raw, type, text } = event;
  const update = raw?.update;
  if (type === "system") return [];
  if (type === "reasoning_summary" || raw?.item?.type === "reasoning") {
    return agentOutput ? [entry("Reasoning summary", text || raw?.item?.text)] : [];
  }
  if (type === "agent_thought_chunk") return agentOutput ? [entry("Reasoning", text)] : [];
  if (type === "subagent_result") {
    if (!agentOutput || event.content == null) return [];
    return [entry("Result", event.content, typeof event.content === "string" ? "message" : "fields")];
  }
  if (agentOutput && type === "command_output") {
    return raw?.chunk ? [entry("Output", raw.chunk, "terminal")] : [];
  }
  if (type === "agent_message_chunk") {
    return [{ ...entry("Agent", text || ""), chunk: type, id: update?.messageId }];
  }
  if (type === "stdout" || type === "stderr") return [entry(type === "stderr" ? "Standard error" : "Output", text || "", "terminal")];
  const item = type === "command_execution" && Array.isArray(raw?.command) ? { ...raw, type } : raw?.item;
  if (["command_execution", "commandExecution"].includes(item?.type)) {
    const rows = [entry("Command", item.command, "command")];
    const output = item.aggregated_output ?? item.aggregatedOutput;
    const exitCode = item.exit_code ?? item.exitCode;
    if (output) rows.push(entry("Output", output, "terminal"));
    if (exitCode != null) rows.push(entry("Exit code", String(exitCode), "plain"));
    return rows;
  }
  if (item?.text) return [entry(item.type === "agent_message" ? "Agent" : label(item.type), item.text)];
  if (Array.isArray(raw?.message?.content)) return raw.message.content
    .filter(block => block?.type !== "redacted_thinking" && (block?.type !== "thinking" || (agentOutput && block.thinking)))
    .map(block => block?.type === "thinking" ? entry("Reasoning", block.thinking) : claudeBlock(block));
  if (type === "tool_call" || type === "tool_call_update") {
    const rows = [entry(update?.title || "Tool", update?.status || update?.kind || "Called", "plain")];
    if (update?.rawInput?.command) rows.push(entry("Command", update.rawInput.command, "command"));
    else if (update?.rawInput) rows.push(entry("Input", update.rawInput, "fields"));
    if (update?.content?.length) {
      for (const block of update.content) {
        if (block.content?.text) rows.push(entry("Output", block.content.text, "terminal"));
        else rows.push(entry("Output", block, "fields"));
      }
    } else if (update?.rawOutput) rows.push(entry("Output", update.rawOutput, typeof update.rawOutput === "string" ? "terminal" : "fields"));
    return rows;
  }
  if (typeof text === "string" && text) return [entry(label(type || "Agent"), text)];
  if (raw?.usage) return [entry(label(type), raw.usage, "fields")];
  if (type === "session" || type === "thread.started") return [entry("Session started", event.session || raw?.thread_id, "plain")];
  const details = Object.fromEntries(Object.entries(item || update || raw || event).filter(([k]) =>
    !["type", "session", "text", "raw", "sessionUpdate", "logged_at"].includes(k)));
  return [entry(label(type || "Event"), Object.keys(details).length ? details : "", "fields")];
}

function logEntries(source) {
  const entries = [];
  const append = (row) => {
    const previous = entries.at(-1);
    if (row.chunk && previous?.chunk === row.chunk && previous.id === row.id) previous.body += row.body;
    else entries.push(row);
  };
  const lines = source.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "").split(/\r?\n/);
  if (lines.at(-1) === "") lines.pop();
  for (const line of lines) {
    if (line.startsWith("[check] ")) {
      const args = line.slice(8);
      const argv = parsed(args);
      append(entry("Check", Array.isArray(argv) ? argv : args, "command"));
    } else if (line.startsWith("[case] ")) {
      append(entry("Case", line.slice(7), "plain"));
    } else {
      const event = line.startsWith("{") && parsed(line);
      if (event && typeof event === "object") agentEvents(event).forEach(append);
      else if (entries.at(-1)?.kind === "terminal" && entries.at(-1)?.title === "Output") entries.at(-1).body += "\n" + line;
      else append(entry("Output", line, "terminal"));
    }
  }
  return entries;
}

function MessageBody({ text }) {
  const object = parsed(text);
  if (object && typeof object === "object") return <Fields value={object} />;
  return <Markdown>{text}</Markdown>;
}

export function LogBody({body,kind}) {
  return <>{body !== "" && body != null && (kind === "command" ? <Command argv={body} /> :
        kind === "terminal" ? <pre tabIndex={0}>{body}</pre> : kind === "fields" ? <Fields value={body} /> :
        kind === "plain" ? <p>{body}</p> : <MessageBody text={body} />)}</>;
}

export function messageRows(record, agentOutput) {
  return record.event ? agentEvents(record.event, agentOutput) : logEntries(record.line || "");
}
export function TurnOutput({ source, name = "Turn log" }) {
  const rows = useMemo(() => logEntries(source || ""), [source]);
  const [visible, setVisible] = useState(80);
  return <section aria-label={name} className="turn-output">
    {!rows.length && <p>This turn has no log output yet.</p>}
    {rows.slice(0, visible).map(({ title, body, kind }, i) => <article className={'log-event log-' + kind} key={i}>
      <h4>{title}</h4>
      <LogBody {...{body,kind}} />
    </article>)}
    {visible < rows.length && <Button variant="outline" onClick={() => setVisible(visible + 80)}>Show more events</Button>}
  </section>;
}

// Git uses C quoting for paths containing tabs, non-ASCII bytes or quotes.
function gitPath(path) {
  path = path.trimEnd();
  if (path.startsWith('"') && path.endsWith('"')) {
    const bytes = [];
    const encoder = new TextEncoder();
    for (const part of path.slice(1, -1).match(/\\[0-7]{1,3}|\\.|[^\\]+/g) || []) {
      if (/^\\[0-7]/.test(part)) bytes.push(parseInt(part.slice(1), 8));
      else {
        const value = part.startsWith("\\") ? ({ t: "\t", n: "\n", r: "\r" }[part[1]] ?? part[1]) : part;
        for (const byte of encoder.encode(value)) bytes.push(byte);
      }
    }
    path = new TextDecoder().decode(Uint8Array.from(bytes));
  }
  return path.replace(/^[ab]\//, "");
}

function readDiff(text) {
  const files = [];
  let file, oldLine = 0, newLine = 0, hunk = false;
  for (const line of text.replace(/\n$/, "").split("\n")) {
    if (line.startsWith("diff --git ")) {
      const parts = line.slice(11).match(/^(?:"(?:\\.|[^"])*"|a\/.+?) ("(?:\\.|[^"])*"|b\/.+)$/);
      file = { path: gitPath(parts?.[1] || line.slice(11)), lines: [], added: 0, removed: 0 };
      files.push(file);
      hunk = false;
      continue;
    }
    if (!file) { file = { path: "Changes", lines: [], added: 0, removed: 0 }; files.push(file); }
    const range = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
    let kind = "meta", left = "", right = "";
    if (range) {
      oldLine = Number(range[1]); newLine = Number(range[2]); hunk = true;
      kind = "hunk";
    } else if (hunk && /^[+ -]/.test(line)) {
      if (!line.startsWith("+")) left = oldLine++;
      if (!line.startsWith("-")) right = newLine++;
      kind = line[0] === "+" ? "addition" : line[0] === "-" ? "deletion" : "context";
      if (kind === "addition") file.added++;
      if (kind === "deletion") file.removed++;
    } else if (!hunk && /^(--- |\+\+\+ )/.test(line)) {
      const path = gitPath(line.slice(4));
      if (path !== "/dev/null") file.path = path;
      continue;
    } else if (!hunk && line.startsWith("index ")) continue;
    file.lines.push({ text: line, kind, left, right });
  }
  return files;
}

export function DiffExplorer({ text }) {
  const files = useMemo(() => text ? readDiff(text) : [], [text]);
  const [selected, setSelected] = useState(0);
  const [limit, setLimit] = useState(300);
  const file = files[selected];
  if (!file) return <p>No file changes.</p>;
  return <section aria-label="Candidate diff" className="diff-explorer">
    <div className="diff-toolbar"><Label>Changed file<NativeSelect value={selected} onChange={(event) => { setSelected(Number(event.target.value)); setLimit(300); }}>
      {files.map((f, i) => <NativeSelectOption value={i} key={i}>{f.path}</NativeSelectOption>)}</NativeSelect></Label>
      <span aria-label={file.added + ' additions, ' + file.removed + ' deletions'}><b>+{file.added}</b> −{file.removed}</span></div>
    <div className="diff-lines" role="region" aria-label={'Changes to ' + file.path} tabIndex={0}>
      {file.lines.slice(0, limit).map((row, i) => <div key={i} className={'diff-line ' + row.kind}>
        <span>{row.left}</span><span>{row.right}</span><code>{row.text || " "}</code></div>)}
    </div>
    {limit < file.lines.length && <Button variant="outline" onClick={() => setLimit(limit + 300)}>Show more lines</Button>}
  </section>;
}
