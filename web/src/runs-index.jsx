import React from "react";
import { ArrowDown, ArrowUp, ChevronsUpDown, Plus } from "lucide-react";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./components/ui/table";
import { FilterChips } from "./filter-chips";
import { runProjectPath, runTitle } from "./run-summary";
import { runStatus, Status, statusLabel } from "./status";
import { changeRoute, destination } from "./navigation";

const headings = {
  title: "Run",
  state: "Status",
  milestones: "Completed",
  tokens: "Settled tokens",
  started: "Started",
};
const compareText = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const date = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

export function RunsIndex({ runs, navigate, route }) {
  const column = Object.hasOwn(headings, route.sort) ? route.sort : "started";
  const descending = route.order === "desc" || (route.order !== "asc" && column === "started");
  const query = route.q || "";
  const search = query.trim().toLowerCase();
  const status = route.status || "";
  const allRows = runs.map((run) => {
    const tasks = run.tasks || [];
    const status = runStatus(run);
    return {
      run,
      title: runTitle(run),
      project: runProjectPath(run),
      status,
      state: statusLabel(status),
      milestones: tasks.filter((t) => t.state === "committed").length,
      total: tasks.filter(t => !["abandoned", "superseded"].includes(t.state)).length,
      tokens: run.tokens,
      started: Date.parse(run.started) || null,
    };
  });
  const statuses = [...new Set(allRows.map((row) => row.status))];
  // Keep the selected status available when a live update removes its last matching run.
  if (status && !statuses.includes(status)) statuses.push(status);
  statuses.sort((a, b) => compareText.compare(statusLabel(a), statusLabel(b)));
  const rows = allRows
    .filter(
      (row) =>
        (!status || row.status === status) &&
        [row.title, row.project].some((value) => value.toLowerCase().includes(search)),
    )
    .sort((a, b) => {
      const x = a[column],
        y = b[column];
      if (x == null || y == null) return Number(x == null) - Number(y == null);
      return (typeof x === "number" ? x - y : compareText.compare(x, y)) * (descending ? -1 : 1);
    });
  return (
    <section className="runs-index">
      <header>
        <div>
          <h1 tabIndex={-1}>Runs</h1>
          <span>
            {search || status ? `${rows.length} of ` : ""}
            {runs.length} {runs.length === 1 ? "run" : "runs"}
          </span>
        </div>
        <Button className="primary" onClick={() => navigate("")}>
          <Plus />
          New run
        </Button>
      </header>
      <div className="filter-controls">
        <Input
          type="search"
          aria-label="Search runs by name or project"
          placeholder="Search by run name or project…"
          value={query}
          onChange={(event) => {
            const q = event.target.value;
            changeRoute({ q }, { replace: !!query && !!q });
          }}
        />
        <NativeSelect
          aria-label="Run status"
          value={status}
          onChange={(event) => changeRoute({ status: event.target.value })}
        >
          <NativeSelectOption value="">All statuses</NativeSelectOption>
          {statuses.map((value) => (
            <NativeSelectOption key={value} value={value}>
              {statusLabel(value)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </div>
      <FilterChips
        filters={[
          ["q", "Search", query],
          ["status", "Status", statusLabel(status)],
        ]}
        onRemove={(key) => changeRoute({ [key]: null })}
      />
      {rows.length > 0 && (
        <Table aria-label="All runs">
          <TableHeader>
            <TableRow>
              {Object.entries(headings).map(([key, label]) => {
                const active = column === key,
                  Icon = active ? (descending ? ArrowDown : ArrowUp) : ChevronsUpDown;
                return (
                  <TableHead
                    key={key}
                    scope="col"
                    aria-sort={active ? (descending ? "descending" : "ascending") : "none"}
                  >
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() =>
                        changeRoute({
                          sort: key,
                          order: (active ? !descending : ["started", "tokens", "milestones"].includes(key))
                            ? "desc"
                            : "asc",
                        })
                      }
                    >
                      {label}
                      <Icon />
                    </Button>
                  </TableHead>
                );
              })}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row) => (
              <TableRow
                key={row.run.id}
                className="open-row"
                onClick={(e) => {
                  if (!e.target.closest("a")) navigate(row.run.id);
                }}
              >
                <TableCell>
                  <a
                    href={"?" + new URLSearchParams(destination(row.run.id))}
                    onClick={(e) => {
                      if (!e.ctrlKey && !e.metaKey && !e.shiftKey && !e.altKey) {
                        e.preventDefault();
                        navigate(row.run.id);
                      }
                    }}
                  >
                    {row.title}
                  </a>
                  <small title={row.project}>
                    {[row.project.split("/").filter(Boolean).at(-1), row.run.input?.workers?.join(", ")]
                      .filter(Boolean)
                      .join(" · ")}
                  </small>
                </TableCell>
                <TableCell>
                  <Status run={row.run} state={row.run.state} />
                </TableCell>
                <TableCell>
                  {row.milestones} / {row.total}
                </TableCell>
                <TableCell>{row.tokens?.toLocaleString() ?? "—"}</TableCell>
                <TableCell>
                  {row.started ? <time dateTime={row.run.started}>{date.format(row.started)}</time> : "—"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
      {rows.length === 0 && <p role="status">No matching runs.</p>}
    </section>
  );
}
