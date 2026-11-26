import React, { useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, ListFilter, X } from "lucide-react";
import { Button } from "./components/ui/button";
import { Badge } from "./components/ui/badge";
import { Input } from "./components/ui/input";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./components/ui/table";
import { Dialog } from "./dialog";
import { Loading, Status } from "./status";
import { LogTime, TurnLog } from "./turn-log";
import { shortSummary } from "./run-summary";

import { changeRoute } from "./navigation";

export const activityRoles = {
  plan: "Planning",
  advisor: "Advisor",
  observe: "Coordinator review",
  worker: "Worker",
  eval: "Evaluation",
  chat: "Conversation",
  repair: "Repair planning",
  release: "Release",
  pricing: "Model pricing",
};
function Choice({ label, value, options, onChange }) {
  return (
    <NativeSelect aria-label={label} value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map(([key, name]) => (
        <NativeSelectOption key={key} value={key}>
          {name}
        </NativeSelectOption>
      ))}
    </NativeSelect>
  );
}
function name(row) {
  return shortSummary(row.detail, 110) || activityRoles[row.role] || row.role;
}
export function ActivityLog({ run, api, explain, route }) {
  const query = {
    q:route.q || "", role:route.role || "", agent:route.agent || "", state:route.state || "",
    sort:route.sort === "oldest" ? "oldest" : "newest",
    page:Math.max(1, parseInt(route.logPage, 10) || 1), size:[10,20,50].includes(Number(route.size)) ? Number(route.size) : 20,
  };
  const [data, setData] = useState(null),
    [error, setError] = useState(""),
    [retry, setRetry] = useState(0);
  const [now, setNow] = useState(Date.now);
  const params = new URLSearchParams(query).toString();
  const result = data?.params === params ? data : null;
  useEffect(() => {
    const clock = setInterval(() => setNow(Date.now()), 15000);
    return () => clearInterval(clock);
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    setError("");
    api("/runs/" + run.id + "/activity?" + params, undefined, controller.signal)
      .then((page) => {
        if (!controller.signal.aborted) setData({ ...page, params });
      })
      .catch((e) => {
        if (!controller.signal.aborted) setError(e.message);
      });
    return () => controller.abort();
  }, [run.id, run.revision, api, params, retry]);
  function filter(key, value) {
    changeRoute({[key]:value, logPage:null, turn:null}, {replace:key === "q" && !!query.q && !!value});
  }
  const turns = run.turns || [],
    agents = [
      ...new Set([run.input.coordinator, ...run.input.workers, ...turns.map((t) => t.agent)].filter(Boolean)),
    ];
  const stateOptions = [...new Set(turns.map((t) => t.state))].map((s) => [s, s]);
  const selected = route.turn && (result?.items.find(row => row.id === route.turn) || turns.find(row => row.id === route.turn));
  const select = row => changeRoute({turn:row?.id});
  const chips = [["q","Search",query.q], ["role","Type",query.role === "coordinator" ? "Coordinator activity" : activityRoles[query.role] || query.role],
    ["agent","Agent",query.agent], ["state","Status",query.state]].filter(([, , value]) => value);
  return (
    <section className="activity-log">
      <header className="work-section-heading">

        {result && (
          <span>
            {result.total} {result.total === 1 ? "turn" : "turns"}
          </span>
        )}
      </header>
      <div className="activity-filters">
        <Input
          type="search"
          aria-label="Search activity"
          placeholder="Search activity…"
          value={query.q}
          onChange={(e) => filter("q", e.target.value)}
        />
        <Choice
          label="Activity type"
          value={query.role}
          options={[["", "All types"], ["coordinator", "Coordinator activity"], ...Object.entries(activityRoles)]}
          onChange={(v) => filter("role", v)}
        />
        <Choice
          label="Activity agent"
          value={query.agent}
          options={[["", "All agents"], ...agents.map((a) => [a, a])]}
          onChange={(v) => filter("agent", v)}
        />
        <Choice
          label="Activity status"
          value={query.state}
          options={[["", "All statuses"], ...stateOptions]}
          onChange={(v) => filter("state", v)}
        />
        <Choice
          label="Activity order"
          value={query.sort}
          options={[
            ["newest", "Newest first"],
            ["oldest", "Oldest first"],
          ]}
          onChange={(v) => filter("sort", v)}
        />
      </div>
      {chips.length > 0 && <div className="filter-chips" role="group" aria-label="Selected filters">
        {chips.map(([key,label,value]) => <Button key={key} size="sm" variant="secondary" aria-label={`Remove ${label.toLowerCase()} filter ${value}`} onClick={() => filter(key,"")}>
          <span>{label}</span>{value}<X aria-hidden="true"/></Button>)}
      </div>}
      {error && (
        <p role="alert">
          {error}{" "}
          <Button variant="outline" onClick={() => setRetry((n) => n + 1)}>
            Try again
          </Button>
        </p>
      )}
      {!result && !error && <Loading>Loading activity…</Loading>}
      {result &&
        (result.items.length ? (
          <Table aria-label="Run activity">
            <TableHeader>
              <TableRow>
                {["Activity", "Agent", "Status", "Started"].map((k) => (
                  <TableHead key={k}>{k}</TableHead>
                ))}
              </TableRow>
            </TableHeader>
            <TableBody>
              {result.items.map((row) => (
                <TableRow key={row.id} className="open-row" tabIndex={0} aria-haspopup="dialog"
                  aria-label={"View log for " + name(row)}
                  onClick={event => {event.currentTarget.focus({preventScroll:true}); select(row);}}
                  onKeyDown={event => {
                    if (event.key === "Enter" || event.key === " ") {event.preventDefault(); select(row);}
                  }}>
                  <TableCell><strong>{name(row)}</strong><Badge variant="secondary" className="activity-type">{activityRoles[row.role] || row.role}</Badge></TableCell>
                  <TableCell>{row.agent || "—"}</TableCell>
                  <TableCell>
                    <Status state={row.state} />
                  </TableCell>
                  <TableCell>
                    <LogTime at={row.started} now={now} />
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        ) : (
          <p className="activity-empty">
            <ListFilter aria-hidden="true" />
            {turns.length ? "No matching activity." : "Activity appears when the first turn starts."}
          </p>
        ))}
      {result && (
        <nav className="activity-pages" aria-label="Activity pages">
          <Choice
            label="Turns per page"
            value={query.size}
            options={[
              [10, "10 per page"],
              [20, "20 per page"],
              [50, "50 per page"],
            ]}
            onChange={(v) => filter("size", Number(v))}
          />
          <Button
            size="sm"
            variant="outline"
            disabled={result.page === 1}
            onClick={() => changeRoute({logPage:result.page - 1})}
          >
            <ChevronLeft />
            Previous
          </Button>
          <span>
            Page {result.page} of {result.pages}
          </span>
          <Button
            size="sm"
            variant="outline"
            disabled={result.page >= result.pages}
            onClick={() => changeRoute({logPage:result.page + 1})}
          >
            Next
            <ChevronRight />
          </Button>
        </nav>
      )}
      {selected && (
        <Dialog onExplain={() => explain("activity", selected.id)} title={name(selected)} className="evidence-dialog" onClose={() => select(null)}>
          <TurnLog
            key={selected.id}
            api={api}
            path={"/runs/" + run.id + "/turns/" + selected.id + "/log"}
            running={turns.find((t) => t.id === selected.id)?.state === "running"}
          />
        </Dialog>
      )}
    </section>
  );
}
