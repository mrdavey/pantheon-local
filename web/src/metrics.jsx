import React, { useEffect, useMemo, useRef, useState } from "react";
import { Activity, Pause, Play } from "lucide-react";
import { Button } from "./components/ui/button";
import { Card } from "./components/ui/card";
import { Label } from "./components/ui/label";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { Table, TableHead, TableHeader, TableRow, TableBody, TableCell } from "./components/ui/table";
import { Loading } from "./status";
import "./metrics.css";

const number = new Intl.NumberFormat();
const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
const dollars = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 6,
});
const roleNames = {
  plan: "Planning",
  observe: "Coordinator review",
  repair: "Repair",
  chat: "Chat",
  worker: "Worker",
};
const counts = () => ({ tokens: null, usd: null });
function add(target, value, key) {
  if (value != null) target[key] = (target[key] ?? 0) + value;
}
function format(value, key) {
  return value == null ? "—" : (key === "usd" ? dollars : number).format(value);
}

function Select({ label, value, set, options }) {
  return (
    <Label>
      {label}
      <NativeSelect aria-label={label} value={value} onChange={(e) => set(e.target.value)}>
        {options.map(([key, text]) => (
          <NativeSelectOption key={key} value={key}>
            {text}
          </NativeSelectOption>
        ))}
      </NativeSelect>
    </Label>
  );
}
function options(turns, field, all) {
  const values = turns.flatMap((t) => (field === "model" ? modelsOf(t) : [t[field]]));
  return [
    ["", all],
    ...[...new Set(values)]
      .filter(Boolean)
      .sort()
      .map((id) => [id, field === "role" ? roleNames[id] || id : id]),
  ];
}
function modelsOf(turn) {
  const latest = turn.samples.at(-1);
  return latest ? Object.keys(latest.models) : [turn.model];
}

// Each sample replaces that turn's previous counters. Range filters apply to
// the change at each receipt, including corrections from the final receipt.
function summarize(turns, filters, start, end) {
  const total = counts(),
    groups = new Map(),
    events = [],
    included = new Set(),
    missingCost = new Set(),
    missingTokens = new Set();
  for (const t of turns) {
    if ((filters.agent && t.agent !== filters.agent) || (filters.role && t.role !== filters.role)) continue;
    for (const model of modelsOf(t)) {
      if (filters.model && filters.model !== model) continue;
      const key = JSON.stringify([t.agent, model]);
      const prior = counts(),
        range = counts();
      let seen = false;
      for (const sample of t.samples) {
        const reading = sample.models[model];
        if (!reading) continue;
        const change = counts();
        for (const field of ["tokens", "usd"]) {
          if (reading[field] == null) continue;
          change[field] = reading[field] - (prior[field] ?? 0);
          prior[field] = reading[field];
        }
        if (sample.at < start || sample.at > end) continue;
        seen = true;
        events.push({ at: sample.at, ...change });
        for (const field of ["tokens", "usd"]) add(range, change[field], field);
      }
      if (
        !seen &&
        (t.samples.length ||
          (start > 0 && t.state !== "running" && !(t.started >= start && t.started <= end)))
      )
        continue;
      included.add(t.id);
      if (range.usd == null) missingCost.add(t.id);
      if (range.tokens == null) missingTokens.add(t.id);
      const group = groups.get(key) || {
        agent: t.agent,
        model,
        turns: new Set(),
        ...counts(),
        missingCost: false,
      };
      group.turns.add(t.id);
      group.missingCost ||= range.usd == null;
      for (const field of ["tokens", "usd"]) {
        add(group, range[field], field);
        add(total, range[field], field);
      }
      groups.set(key, group);
    }
  }
  return {
    total,
    groups: [...groups.values()].sort((a, b) => (b.tokens || 0) - (a.tokens || 0)),
    events,
    turns: included.size,
    missingCost: missingCost.size,
    missingTokens: missingTokens.size,
  };
}

function UsageChart({ events, field, mode, start, end }) {
  const bins = Array(24).fill(0);
  const span = Math.max(1, end - start);
  for (const event of events) {
    const i = Math.min(23, Math.max(0, Math.floor((24 * (event.at - start)) / span)));
    bins[i] += event[field] ?? 0;
  }
  if (mode === "total") for (let i = 1; i < bins.length; i++) bins[i] += bins[i - 1];
  const min = Math.min(0, ...bins),
    max = Math.max(...bins, field === "tokens" ? 1 : 0.000001);
  const y = (n) => 144 - (130 * (n - min)) / (max - min);
  const x = (i) => 49 + (438 * i) / 24;
  const clock = new Intl.DateTimeFormat(
    undefined,
    span >= 86400000
      ? { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }
      : { hour: "2-digit", minute: "2-digit", ...(span < 60000 ? { second: "2-digit" } : {}) },
  );
  const line = bins.map((v, i) => "H " + x(i + 1) + " V " + y(v)).join(" ");
  return (
    <figure className="usage-chart">
      <svg
        viewBox="0 0 500 153"
        role="img"
        aria-label={`${field === "usd" ? "Estimated cost" : "Token usage"} over time, ${mode === "total" ? "cumulative" : "per interval"}`}
      >
        {[max, (max + min) / 2, min].map((v, i) => (
          <g key={i}>
            <line x1="49" x2="487" y1={y(v)} y2={y(v)} className="usage-rule" />
            <text x="43" y={y(v) + 3} textAnchor="end">
              {field === "usd" ? Number(v.toPrecision(2)).toString() : compact.format(v)}
            </text>
          </g>
        ))}
        {mode === "total" ? (
          <>
            <path className="usage-fill" d={`M 49 ${y(0)} ${line} V ${y(0)} Z`} />
            <path className="usage-line" d={`M 49 ${y(0)} ${line}`} />
            <circle cx="487" cy={y(bins.at(-1))} r="3" />
          </>
        ) : (
          bins.map((v, i) => (
            <rect
              key={i}
              x={x(i) + 2}
              y={Math.min(y(v), y(0))}
              width="14"
              height={Math.abs(y(v) - y(0))}
              rx="2"
            >
              <title>
                {clock.format(start + (span * i) / 24)} · {format(v, field)}
              </title>
            </rect>
          ))
        )}
      </svg>
      <figcaption>
        <time>{clock.format(start)}</time>
        <time>{clock.format(end)}</time>
      </figcaption>
      <details>
        <summary>Chart data</summary>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Through</TableHead>
              <TableHead>{field === "usd" ? "USD" : "Tokens"}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {bins.map((v, i) => (
              <TableRow key={i}>
                <TableCell>{clock.format(start + (span * (i + 1)) / 24)}</TableCell>
                <TableCell>{format(v, field)}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </details>
    </figure>
  );
}

export function Metrics({ run, api }) {
  const [data, setData] = useState(null),
    [error, setError] = useState(""),
    [paused, setPaused] = useState(false);
  const [retry, setRetry] = useState(0),
    [agent, setAgent] = useState(""),
    [model, setModel] = useState(""),
    [role, setRole] = useState("");
  const [range, setRange] = useState("all"),
    [field, setField] = useState("tokens"),
    [mode, setMode] = useState("total");
  const terminal =
    ["landed", "stopped"].includes(run.state) && !run.turns?.some((t) => t.state === "running");
  const turnCount = run.turns?.length || 0;
  const working = run.turns?.some((t) => t.state === "running") || false;
  const cache = useRef({ merged: new Map(), cursor: 0 });
  useEffect(() => {
    if (paused) return;
    const controller = new AbortController();
    const store = cache.current;
    let timer;
    async function update() {
      try {
        const reply = await api(`/runs/${run.id}/metrics?from=${store.cursor}`, undefined, controller.signal);
        if (controller.signal.aborted) return;
        for (const turn of reply.turns) store.merged.set(turn.id, turn);
        store.cursor = reply.next;
        setData({ turns: [...store.merged.values()], now: Date.now() });
        setError("");
      } catch (e) {
        if (!controller.signal.aborted) setError(e.message);
      }
      if (working && !controller.signal.aborted) timer = setTimeout(update, 2500);
    }
    update();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [api, run.id, paused, terminal, retry, turnCount, working]);
  const turns = data?.turns || [];
  const window = useMemo(() => {
    let first = Infinity,
      last = 0;
    for (const t of turns) {
      if (t.started) {
        first = Math.min(first, t.started);
        last = Math.max(last, t.started);
      }
      if (t.samples.length) {
        first = Math.min(first, t.samples[0].at);
        last = Math.max(last, t.samples.at(-1).at);
      }
    }
    const end = terminal && last ? last : data?.now || Date.now();
    const start = range === "all" ? 0 : end - Number(range) * 60000;
    return { start, end, axisStart: start || Math.min(first, end - 1000) };
  }, [data, range, terminal, turns]);
  const summary = useMemo(
    () => summarize(turns, { agent, model, role }, window.start, window.end),
    [turns, agent, model, role, window],
  );
  const legacyTotal =
    range === "all" &&
    !agent &&
    !model &&
    !role &&
    summary.total.tokens == null &&
    run.tokens > 0 &&
    turns.every((turn) => !turn.started && !turn.samples.length)
      ? run.tokens
      : null;
  return (
    <Card className="run-metrics" aria-label="Run metrics">
      <header>
        <h2>
          <Activity aria-hidden="true" />
          Run metrics
        </h2>
        {(!terminal || paused) && (
          <Button variant="outline" size="sm" disabled={!data} onClick={() => setPaused(!paused)}>
            {paused ? <Play /> : <Pause />}
            {paused ? "Resume live updates" : "Pause live updates"}
          </Button>
        )}
      </header>
      {error && (
        <div role="alert">
          {error}{" "}
          <Button
            variant="outline"
            onClick={() => {
              setPaused(false);
              setRetry((n) => n + 1);
            }}
          >
            Retry metrics
          </Button>
        </div>
      )}
      {!data ? (
        !error && <Loading>Loading usage…</Loading>
      ) : (
        <>
          <div className="usage-filters">
            <Select
              label="Time range"
              value={range}
              set={setRange}
              options={[
                ["all", "Entire run"],
                ["15", "Last 15 minutes"],
                ["60", "Last hour"],
                ["1440", "Last 24 hours"],
              ]}
            />
            <Select
              label="Agent"
              value={agent}
              set={setAgent}
              options={options(turns, "agent", "All agents")}
            />
            <Select
              label="Model"
              value={model}
              set={setModel}
              options={options(turns, "model", "All models")}
            />
            <Select label="Role" value={role} set={setRole} options={options(turns, "role", "All roles")} />
          </div>
          <dl className="usage-totals">
            {[
              ["Tokens used", format(summary.total.tokens ?? legacyTotal, "tokens")],
              ["Estimated cost (USD)", format(summary.total.usd, "usd")],
              ["Agent turns", summary.turns],
            ].map(([label, value]) => (
              <div key={label}>
                <dt>{label}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
          {legacyTotal != null && (
            <p className="usage-note">
              This older run has a saved token total. Its per-turn receipts are unavailable.
            </p>
          )}
          {(summary.missingCost > 0 || summary.missingTokens > 0) && (
            <p className="usage-note">
              {[
                summary.missingTokens > 0 &&
                  `Tokens unavailable for ${summary.missingTokens} turn${summary.missingTokens === 1 ? "" : "s"}`,
                summary.missingCost > 0 &&
                  `Cost unavailable for ${summary.missingCost} turn${summary.missingCost === 1 ? "" : "s"}`,
              ]
                .filter(Boolean)
                .join(" · ")}
            </p>
          )}
          <section>
            <h3>Usage over time</h3>
            <div className="usage-chart-controls">
              <Select
                label="Metric"
                value={field}
                set={setField}
                options={[
                  ["tokens", "Tokens"],
                  ["usd", "Cost (USD)"],
                ]}
              />
              <Select
                label="Chart view"
                value={mode}
                set={setMode}
                options={[
                  ["total", "Cumulative"],
                  ["interval", "Per interval"],
                ]}
              />
            </div>
            {summary.events.some((e) => e[field] != null) ? (
              <UsageChart
                events={summary.events}
                field={field}
                mode={mode}
                start={window.axisStart}
                end={window.end}
              />
            ) : (
              <p className="usage-empty">
                {summary.turns ? "No readings for this metric yet." : "No usage in this selection."}
              </p>
            )}
          </section>
          <details className="usage-method">
            <summary>How cost is calculated</summary>
            <p>
              Claude and compatible adapters supply a USD estimate. For supported Codex models, Pantheon
              applies standard API rates to input, cached input, and output tokens. Your subscription, service
              tier, and invoice can differ. Missing costs are excluded.
            </p>
          </details>
          {!!summary.groups.length && (
            <section>
              <h3>Usage by agent and model</h3>
              <Table aria-label="Usage by agent and model">
                <TableHeader>
                  <TableRow>
                    {["Agent / model", "Turns", "Tokens", "Est. USD"].map((k) => (
                      <TableHead key={k}>{k}</TableHead>
                    ))}
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {summary.groups.map((g) => (
                    <TableRow key={JSON.stringify([g.agent, g.model])}>
                      <TableCell>
                        {g.agent}
                        <small>{g.model}</small>
                      </TableCell>
                      <TableCell>{g.turns.size}</TableCell>
                      <TableCell>{format(g.tokens, "tokens")}</TableCell>
                      <TableCell>
                        {format(g.usd, "usd")}
                        {g.missingCost && g.usd != null && " (partial)"}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </section>
          )}
        </>
      )}
    </Card>
  );
}
