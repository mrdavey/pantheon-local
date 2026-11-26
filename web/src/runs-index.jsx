import React from "react";
import { ArrowDown, ArrowUp, ChevronsUpDown, Plus } from "lucide-react";
import { Button } from "./components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./components/ui/table";
import { blockedTasks, runTitle } from "./run-summary";
import { Status, statusLabel } from "./status";
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
  const rows = runs
    .map((run) => {
      const tasks = run.tasks || [];
      return {
        run,
        title: runTitle(run),
        state:
          blockedTasks(run).length && run.state === "working" ? "Milestone blocked" : statusLabel(run.state),
        milestones: tasks.filter((t) => t.state === "committed").length,
        total: tasks.length,
        tokens: run.tokens,
        started: Date.parse(run.started) || null,
      };
    })
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
            {runs.length} {runs.length === 1 ? "run" : "runs"}
          </span>
        </div>
        <Button className="primary" onClick={() => navigate("")}>
          <Plus />
          New run
        </Button>
      </header>
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
                <small>{row.run.input?.workers?.join(", ")}</small>
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
    </section>
  );
}
