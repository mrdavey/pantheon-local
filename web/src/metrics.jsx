import {useRoute, changeRoute} from "./navigation";
import { Disclosure } from "./disclosure";
import React, { useEffect, useMemo, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { Input } from "./components/ui/input";
import { Button } from "./components/ui/button";
import { Label } from "./components/ui/label";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { Table, TableHead, TableHeader, TableRow, TableBody, TableCell } from "./components/ui/table";
import { Loading } from "./status";
import { formatDuration } from "./duration";
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
  pricing: "Price lookup",
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
  return latest ? [...new Set(turn.samples.flatMap((sample) => Object.keys(sample.models)))] : [turn.model];
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
        events.push({ at: sample.at, series: key, agent: t.agent, model, ...change });
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

const usageColors = ["#5656bd", "#147c83", "#a85c27", "#a0447a", "#507b34", "#4575aa"];
const binCount = 24;

function usagePlot(events, field, mode, start, span) {
  const totals = Array(binCount).fill(0);
  const stacks = Array.from({ length: binCount }, () => new Map());
  const series = new Map();
  for (const event of events) {
    const value = event[field];
    if (value == null) continue;
    const bin = Math.min(binCount - 1, Math.max(0, Math.floor((binCount * (event.at - start)) / span)));
    totals[bin] += value;
    if (mode === "interval") {
      series.set(event.series, event);
      const stack = stacks[bin];
      stack.set(event.series, (stack.get(event.series) ?? 0) + value);
    }
  }
  if (mode === "total") {
    for (let bin = 1; bin < totals.length; bin++) totals[bin] += totals[bin - 1];
    return { totals, segments: [], min: Math.min(0, ...totals), max: Math.max(...totals) };
  }
  const ordered = [...series.keys()].sort();
  const segments = [];
  let min = 0, max = 0;
  stacks.forEach((stack, bin) => {
    let positive = 0, negative = 0;
    ordered.forEach((key, index) => {
      const value = stack.get(key);
      if (!value) return;
      const from = value > 0 ? positive : negative;
      const to = from + value;
      if (value > 0) positive = to;
      else negative = to;
      const { agent, model } = series.get(key);
      segments.push({ id: `${bin}:${key}`, bin, agent, model, value, from, to, color: usageColors[index % usageColors.length] });
    });
    min = Math.min(min, negative);
    max = Math.max(max, positive);
  });
  return { totals, segments, min, max };
}

function UsageChart({ events, field, mode, start, end, selectRange }) {
  const [drag, setDrag] = useState(null);
  const [inspection, setInspection] = useState(null);
  const [tabStop, setTabStop] = useState(null);
  const pressed = useRef(null);
  function inspect(element, source) {
    if (!element) {
      setInspection(null);
      return;
    }
    const plot = element.ownerSVGElement.parentElement.getBoundingClientRect();
    const rect = element.getBoundingClientRect();
    const below = rect.top < 80;
    setInspection({
      id: element.dataset.usageSegment,
      source,
      left: rect.left + rect.width / 2 - plot.left,
      top: (below ? rect.bottom + 6 : rect.top - 6) - plot.top,
      below,
    });
  }
  function fraction(event) {
    const svg = event.currentTarget;
    const point = new DOMPoint(event.clientX, event.clientY).matrixTransform(svg.getScreenCTM().inverse());
    return Math.min(1, Math.max(0, (point.x - 49) / 438));
  }
  const span = Math.max(1, end - start);
  const { totals: bins, segments, min, max: peak } = useMemo(
    () => usagePlot(events, field, mode, start, span), [events, field, mode, start, span],
  );
  const max = Math.max(peak, field === "tokens" ? 1 : 0.000001);
  const y = (n) => 144 - (130 * (n - min)) / (max - min);
  const x = (i) => 49 + (438 * i) / binCount;
  const active = !drag && segments.find(segment => segment.id === inspection?.id);
  const tabbable = segments.some(segment => segment.id === tabStop) ? tabStop : segments[0]?.id;
  const clock = new Intl.DateTimeFormat(
    undefined,
    span >= 86400000
      ? { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }
      : { hour: "2-digit", minute: "2-digit", ...(span < 60000 ? { second: "2-digit" } : {}) },
  );
  const line = bins.map((v, i) => "H " + x(i + 1) + " V " + y(v)).join(" ");
  const segmentValue = segment => `${format(segment.value, field)}${field === "tokens" ? " tokens" : ""}`;
  const intervalTime = bin => clock.format(start + (span * bin) / binCount);
  return (
    <figure className="usage-chart">
      <div className="usage-plot">
        <svg
          viewBox="0 0 500 153"
          onPointerDown={(e) => {
            if (e.button !== 0) return;
            pressed.current = e.target.closest("[data-usage-segment]");
            setInspection(null);
            e.currentTarget.setPointerCapture(e.pointerId);
            const f = fraction(e);
            setDrag([f, f]);
          }}
          onPointerMove={(e) => {
            if (drag) setDrag([drag[0], fraction(e)]);
          }}
          onPointerCancel={() => {
            pressed.current = null;
            setDrag(null);
          }}
          onPointerUp={(e) => {
            if (!drag) return;
            const bounds = [drag[0], fraction(e)].sort((a, b) => a - b);
            if (bounds[1] - bounds[0] > 0.02) {
              selectRange(bounds.map((n) => Math.round(start + n * span)));
            } else if (pressed.current) {
              inspect(pressed.current, e.pointerType === "touch" ? "touch" : "hover");
            }
            pressed.current = null;
            setDrag(null);
          }}
          onKeyDown={(e) => {
            if (!e.target.matches("[data-usage-segment]")) return;
            if (e.key === "Escape") {
              setInspection(null);
            } else if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              inspect(e.target, "focus");
            } else if (["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(e.key)) {
              e.preventDefault();
              const marks = Array.from(e.currentTarget.querySelectorAll("[data-usage-segment]"));
              const direction = ["ArrowLeft", "ArrowDown"].includes(e.key) ? -1 : 1;
              const next = e.key === "Home" ? 0 : e.key === "End" ? marks.length - 1 : marks.indexOf(e.target) + direction;
              marks[next]?.focus();
            }
          }}
          role={mode === "interval" ? "group" : "img"}
          aria-label={`${field === "usd" ? "Estimated cost" : "Token usage"} over time, ${mode === "total" ? "cumulative" : "per interval"}`}
        >
          {[max, mode === "interval" && min < 0 ? 0 : (max + min) / 2, min].map((v, i) => (
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
            segments.map(segment => (
              <rect
                key={segment.id}
                className="usage-segment"
                data-usage-segment={segment.id}
                data-active={active?.id === segment.id || undefined}
                role="img"
                aria-label={`${segment.agent} · ${segment.model}, ${segmentValue(segment)}, ${intervalTime(segment.bin)}`}
                tabIndex={segment.id === tabbable ? 0 : -1}
                fill={segment.color}
                x={x(segment.bin) + 2}
                y={y(Math.max(segment.from, segment.to))}
                width="14"
                height={Math.abs(y(segment.to) - y(segment.from))}
                onPointerEnter={(e) => {
                  if (e.pointerType !== "touch" && !drag) inspect(e.currentTarget, "hover");
                }}
                onPointerLeave={() => setInspection(current => current?.source === "hover" ? null : current)}
                onFocus={(e) => {
                  setTabStop(segment.id);
                  inspect(e.currentTarget, "focus");
                }}
                onBlur={() => setInspection(current => current?.source === "focus" ? null : current)}
              />
            ))
          )}
          {drag && (
            <rect
              className="chart-selection"
              x={49 + 438 * Math.min(...drag)}
              y="0"
              width={438 * Math.abs(drag[1] - drag[0])}
              height="144"
            />
          )}
        </svg>
        {active && (
          <div
            className="usage-segment-tooltip"
            role="tooltip"
            data-below={inspection.below || undefined}
            style={{ "--segment-x": `${inspection.left}px`, top: inspection.top }}
          >
            <strong>{active.agent} · {active.model}</strong>
            <span>{segmentValue(active)} · {intervalTime(active.bin)}</span>
          </div>
        )}
      </div>
      <figcaption>
        <time>{clock.format(start)}</time>
        <time>{clock.format(end)}</time>
      </figcaption>
      <Disclosure title="Chart data">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Through</TableHead>
              {mode === "interval" && <TableHead className="usage-data-series">Agent and model</TableHead>}
              <TableHead>{field === "usd" ? "USD" : "Tokens"}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {bins.map((v, i) => (
              <React.Fragment key={i}>
                <TableRow>
                  <TableCell>{intervalTime(i + 1)}</TableCell>
                  {mode === "interval" && <TableCell className="usage-data-series">Total</TableCell>}
                  <TableCell>{format(v, field)}</TableCell>
                </TableRow>
                {mode === "interval" && segments.filter(segment => segment.bin === i).map(segment => (
                  <TableRow key={segment.id}>
                    <TableCell>{intervalTime(i + 1)}</TableCell>
                    <TableCell className="usage-data-series">{segment.agent} · {segment.model}</TableCell>
                    <TableCell>{format(segment.value, field)}</TableCell>
                  </TableRow>
                ))}
              </React.Fragment>
            ))}
          </TableBody>
        </Table>
      </Disclosure>
    </figure>
  );
}

function ShareChart({ groups, field }) {
  const rows = groups.filter((g) => g[field] > 0);
  const total = rows.reduce((sum, g) => sum + g[field], 0);
  let arc = 0;
  const slices = rows.map((g, i) => {
    const from = arc;
    arc += (100 * g[field]) / total;
    return usageColors[i % usageColors.length] + " " + from + "% " + arc + "%";
  });
  if (!total) return <p className="usage-empty">No positive usage in this selection.</p>;
  return (
    <figure className="usage-shares">
      <div
        className="share-disc"
        role="img"
        aria-label="Usage share by agent and model"
        style={{ background: "conic-gradient(" + slices.join(",") + ")" }}
      />
      <figcaption>
        {rows.map((g, i) => (
          <div key={JSON.stringify([g.agent, g.model])}>
            <i style={{ background: usageColors[i % usageColors.length] }} />
            <span>
              {g.agent} · {g.model}
            </span>
            <strong>{((100 * g[field]) / total).toFixed(1)}%</strong>
          </div>
        ))}
      </figcaption>
    </figure>
  );
}
export function Metrics({ run, api }) {
  const [data, setData] = useState(null),
    [error, setError] = useState(""),
    [paused, setPaused] = useState(false);
  const [retry, setRetry] = useState(0);
  const route = useRoute();
  const agent = route.agent || "", model = route.model || "", role = route.role || "";
  const range = ["15","60","1440"].includes(route.range) ? route.range : "all";
  const field = route.metric === "usd" ? "usd" : "tokens";
  const mode = ["pie","interval"].includes(route.chart) ? route.chart : "total";
  const [zoomOpen, setZoomOpen] = useState(Boolean(route.from && route.to));
  const selection = useMemo(() => {
    const a = Number(route.from), b = Number(route.to);
    return Number.isSafeInteger(a) && a > 0 && Number.isSafeInteger(b) && b > a ? [a,b] : null;
  }, [route.from, route.to]);
  const setSelection = bounds => changeRoute({from:bounds?.[0], to:bounds?.[1]});
  const [fetching, setFetching] = useState(false),
    [lookupMessage, setLookupMessage] = useState("");
  const pricingBusy = run.turns?.some((t) => t.role === "pricing" && t.state === "running");
  const terminal =
    ["landed", "stopped"].includes(run.state) && !run.turns?.some((t) => t.state === "running");
  const turnCount = run.turns?.length || 0;
  const working = run.turns?.some((t) => t.state === "running") || false;
  const cache = useRef({ merged: new Map(), cursor: 0 });
  useEffect(() => {
    if (paused) return;
    const controller = new AbortController();
    const store = cache.current;
    if (store.pricing !== run.pricing_revision) {
      store.pricing = run.pricing_revision;
      store.cursor = 0;
      store.merged.clear();
    }
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
  }, [api, run.id, paused, terminal, retry, turnCount, working, run.state, run.pricing_revision]);
  const turns = data?.turns || [];
  const workingAgents = new Set(turns.filter(turn => turn.state === "running").map(turn => turn.agent));
  const team = [...new Set([run.input.coordinator, ...run.input.workers])].filter(Boolean);
  const roster = team
    .filter((id) => !turns.some((t) => t.agent === id))
    .map((id) => {
      const p = run.profiles?.[id];
      return {
        agent: id,
        model: p?.model || p?.detected?.model || p?.detected?.label || "Harness default",
        samples: [],
      };
    });
  const allAgents = [...turns, ...roster];
  const window = useMemo(() => {
    let first = Date.parse(run.started) || Infinity,
      last = 0;
    for (const t of turns) {
      if (t.role === "pricing") continue;
      if (t.started) {
        first = Math.min(first, t.started);
        last = Math.max(last, t.started);
      }
      if (t.samples.length) {
        first = Math.min(first, t.samples[0].at);
        last = Math.max(last, t.samples.at(-1).at);
      }
    }
    const live =
      !terminal &&
      (run.turns?.some((t) => t.state === "running" && t.role !== "pricing") ||
        (["planning", "working", "evaluating", "repairing"].includes(run.state) &&
          !run.tasks?.some((t) => t.state === "blocked")));
    const endedAt =
      run.state === "landed"
        ? Date.parse(run.selection?.landed_at)
        : run.state === "stopped"
          ? Date.parse(run.stopped_at)
          : null;
    const end = live
      ? data?.now || Date.now()
      : endedAt || Math.max(last, Number.isFinite(first) ? first : 0);
    first = Math.min(first, end);
    const start = range === "all" ? first : Math.max(first, end - Number(range) * 60000);
    const intersects = selection && selection[0] < end && selection[1] > first;
    return {
      first,
      last: end,
      elapsed: end - first,
      start: intersects ? Math.max(first, selection[0]) : start,
      end: intersects ? Math.min(end, selection[1]) : end,
    };
  }, [data, range, terminal, turns, run, selection]);
  const summary = useMemo(
    () => summarize(turns, { agent, model, role }, window.start, window.end),
    [turns, agent, model, role, window],
  );
  const legacyTotal =
    range === "all" &&
    !selection &&
    !agent &&
    !model &&
    !role &&
    summary.total.tokens == null &&
    run.tokens > 0 &&
    turns.every((turn) => !turn.samples.length)
      ? run.tokens
      : null;
  return (
    <section className="run-metrics" aria-label="Run metrics">
      <header>

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
              setError("");
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
              value={selection ? "selection" : range}
              set={(value) => {
                changeRoute({from:null, to:null, range:value});
              }}
              options={[
                ...(selection ? [["selection", "Selected interval"]] : []),
                ["all", "Entire run"],
                ["15", "Last 15 minutes"],
                ["60", "Last hour"],
                ["1440", "Last 24 hours"],
              ]}
            />
            <Select
              label="Agent"
              value={agent}
              set={agent => changeRoute({agent})}
              options={options(allAgents, "agent", "All agents")}
            />
            <Select
              label="Model"
              value={model}
              set={model => changeRoute({model})}
              options={options(allAgents, "model", "All models")}
            />
            <Select label="Role" value={role} set={role => changeRoute({role})} options={options(turns, "role", "All roles")} />
          </div>
          <dl className="usage-totals">
            {[
              ["Tokens used", format(summary.total.tokens ?? legacyTotal, "tokens")],
              ["Estimated cost (USD)", format(summary.total.usd, "usd")],
              ["Agent turns", summary.turns],
              ["Agents working", workingAgents.size],
              ["Elapsed time", formatDuration(window.elapsed)],
            ].map(([label, value]) => (
              <div key={label} title={label === "Agents working" ? "Agents with an active turn across this run" : undefined}>
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
          {summary.missingCost > 0 && (
            <div className="price-actions">
              {fetching && <Loading top>Finding model prices…</Loading>}
              <Button
                variant="outline"
                disabled={fetching || pricingBusy}
                onClick={async () => {
                  setFetching(true);
                  setLookupMessage("");
                  try {
                    const reply = await api(`/runs/${run.id}/metrics/prices`, {});
                    setLookupMessage(reply.status);
                  } catch (err) {
                    setLookupMessage(err.message);
                  } finally {
                    setFetching(false);
                  }
                }}
              >
                {pricingBusy ? "Finding prices…" : "Fetch token prices"}
              </Button>
              <span role="status">
                {pricingBusy
                  ? "Coordinator is checking published rates."
                  : lookupMessage && lookupMessage !== "Finding model prices…"
                    ? lookupMessage
                    : run.pricing_status}
              </span>
            </div>
          )}
          <section>
            <h3>Usage over time</h3>
            <div className="usage-chart-controls">
              <Select
                label="Metric"
                value={field}
                set={metric => changeRoute({metric})}
                options={[
                  ["tokens", "Tokens"],
                  ["usd", "Cost (USD)"],
                ]}
              />
              <Select
                label="Chart view"
                value={mode}
                set={chart => changeRoute({chart})}
                options={[
                  ["total", "Line · cumulative"],
                  ["interval", "Stacked bars"],
                  ["pie", "Pie · share by agent and model"],
                ]}
              />
            </div>
            {window.last > window.first && (
              <Disclosure className="zoom-range" title="Zoom time range" open={zoomOpen} onOpenChange={setZoomOpen}>
                <div>
                  {["From", "To"].map((name, i) => (
                    <Label key={name}>
                      {name} · {new Date(i ? window.end : window.start).toLocaleTimeString()}
                      <Input
                        aria-label={name + " time"}
                        type="range"
                        min={window.first}
                        max={window.last}
                        step={Math.max(1, Math.round((window.last - window.first) / 1000))}
                        value={i ? window.end : window.start}
                        onChange={(e) => {
                          const time = Number(e.target.value);
                          setSelection(
                            i
                              ? [window.start, Math.max(window.start + 1, time)]
                              : [Math.min(time, window.end - 1), window.end],
                          );
                        }}
                      />
                    </Label>
                  ))}
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={!selection}
                    onClick={() => setSelection(null)}
                  >
                    Reset zoom
                  </Button>
                </div>
              </Disclosure>
            )}
            {summary.events.some((e) => e[field] != null) ? (
              mode === "pie" ? (
                <ShareChart groups={summary.groups} field={field} />
              ) : (
                <UsageChart
                  selectRange={range => {setZoomOpen(true); setSelection(range);}}
                  events={summary.events}
                  field={field}
                  mode={mode}
                  start={window.start}
                  end={window.end}
                />
              )
            ) : (
              <p className="usage-empty">
                {summary.turns ? "No readings for this metric yet." : "No usage in this selection."}
              </p>
            )}
          </section>
          <Disclosure className="usage-method" title="How cost is calculated">
            <p>
              Claude and compatible adapters supply a USD estimate. For supported Codex models, Pantheon
              applies standard API rates to input, cached input, and output tokens. Your subscription, service
              tier, and invoice can differ. The coordinator can look up missing rates. Estimates require a
              token breakdown unless every category has the same price. Missing readings appear as a dash.
            </p>
            {Object.entries(run.model_rates || {}).map(([name, rate]) => (
              <p key={name}>
                {name} · {dollars.format(rate.input)} input / {dollars.format(rate.output)} output per million
                tokens ·{" "}
                <a href={rate.source} target="_blank" rel="noreferrer">
                  Pricing source
                </a>{" "}
                · {new Date(rate.fetched).toLocaleDateString()}
              </p>
            ))}
          </Disclosure>
          <section>
            <h3>Usage by agent and model</h3>
            <Table aria-label="Usage by agent and model">
              <TableHeader>
                <TableRow>
                  {["Agent", "Model", "Turns", "Tokens", "Est. USD"].map((k) => (
                    <TableHead key={k}>{k}</TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {[
                  ...summary.groups,
                  ...roster
                    .filter((r) => (!agent || r.agent === agent) && (!model || r.model === model) && !role)
                    .map((r) => ({ ...r, turns: new Set() })),
                ].map((g) => (
                  <TableRow key={JSON.stringify([g.agent, g.model])}>
                    <TableCell>{g.agent}</TableCell>
                    <TableCell>{g.model}</TableCell>
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
        </>
      )}
    </section>
  );
}
