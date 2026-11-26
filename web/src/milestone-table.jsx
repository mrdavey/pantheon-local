import React, { useEffect, useMemo, useState } from "react";
import { ArrowDown, ArrowUp, ChevronsUpDown } from "lucide-react";
import { Button } from "./components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./components/ui/table";
import { Status, statusLabel } from "./status";
import { canRetryMilestone, retryTokens, shortSummary } from "./run-summary";

const columns = ["Milestone", "Owner", "Status", "Completed"];
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

export function MilestoneTable({ run, tasks, api, act, busy, onSelect, card, question }) {
  const [sort, setSort] = useState(null);
  const [receipts, setReceipts] = useState({});
  const [error, setError] = useState(""),
    [retry, setRetry] = useState(0);
  const finished = useMemo(
    () => new Map((run.turns || []).filter((t) => t.role === "worker").map((t) => [t.task, t.finished])),
    [run.turns],
  );
  const missing = tasks
    .filter((t) => t.state === "committed" && t.completed_at == null && finished.get(t.id) == null)
    .map((t) => t.id)
    .join(",");
  useEffect(() => {
    if (!missing) return;
    const controller = new AbortController();
    api("/runs/" + run.id + "/milestones/completions", undefined, controller.signal)
      .then((data) => {
        if (!controller.signal.aborted) {
          setReceipts(data);
          setError("");
        }
      })
      .catch((e) => {
        if (!controller.signal.aborted) setError(e.message);
      });
    return () => controller.abort();
  }, [api, run.id, missing, retry]);
  const rows = tasks.map((task) => ({
    task,
    Milestone: shortSummary(task.goal, 90) || task.id,
    Owner: task.worker || "",
    Status: statusLabel(task.state),
    Completed:
      task.state === "committed" ? (task.completed_at ?? finished.get(task.id) ?? receipts[task.id]) : null,
  }));
  if (sort)
    rows.sort((a, b) => {
      const x = a[sort.key],
        y = b[sort.key];
      if (x == null || y == null) return Number(x == null) - Number(y == null);
      return (sort.key === "Completed" ? x - y : collator.compare(x, y)) * sort.direction;
    });
  return (
    <>
      {error && missing && (
        <p role="alert">
          {error}{" "}
          <Button variant="outline" onClick={() => setRetry((n) => n + 1)}>
            Retry dates
          </Button>
        </p>
      )}
      <Table aria-label="Milestones" className="milestone-table">
        <TableHeader>
          <TableRow>
            {columns.map((key) => {
              const active = sort?.key === key;
              const Icon = active ? (sort.direction === 1 ? ArrowUp : ArrowDown) : ChevronsUpDown;
              return (
                <TableHead
                  key={key}
                  scope="col"
                  aria-sort={active ? (sort.direction === 1 ? "ascending" : "descending") : "none"}
                >
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() =>
                      setSort({ key, direction: active ? -sort.direction : key === "Completed" ? -1 : 1 })
                    }
                  >
                    {key}
                    <Icon />
                  </Button>
                </TableHead>
              );
            })}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map(({ task, Completed }) => (
            <TableRow
              key={task.id}
              className="open-row"
              onClick={(event) => {
                if (!event.target.closest("button,a,form")) {
                  event.currentTarget.querySelector("[data-task]").focus({ preventScroll: true });
                  onSelect(task.id);
                }
              }}
            >
              <TableCell>
                {card(task)}
                {task.question && task.state === "question" && !run.project && question(task)}
              </TableCell>
              <TableCell>{task.worker}</TableCell>
              <TableCell>
                <Status state={task.state} />
                {canRetryMilestone(run, task) && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={busy}
                    onClick={() => act({ kind: "resume", task: task.id, tokens: retryTokens(run) })}
                  >
                    Retry milestone
                  </Button>
                )}
              </TableCell>
              <TableCell>
                {Completed == null ? (
                  <span
                    className="muted"
                    aria-label={task.state === "committed" ? "Completion time unavailable" : "Not completed"}
                  >
                    —
                  </span>
                ) : (
                  <time
                    dateTime={new Date(Completed).toISOString()}
                    title={new Date(Completed).toLocaleString()}
                  >
                    {dateFormat.format(Completed)}
                  </time>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </>
  );
}
