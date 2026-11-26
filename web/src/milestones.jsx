import React, { useEffect, useState } from "react";
import { Columns3, List, Network, Plus } from "lucide-react";
import { Button } from "./components/ui/button";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "./components/ui/table";
import { Textarea } from "./components/ui/textarea";
import { Label } from "./components/ui/label";
import { Dialog } from "./dialog";
import { Markdown } from "./markdown";
import { MilestoneTime } from "./milestone-time";
import { Status } from "./status";
import { canRetryMilestone, retryTokens, shortSummary } from "./run-summary";
import { Proposal, RoutingEvidence } from "./conversation";
import { AssignmentMap } from "./assignment-map";

export function MilestoneDetails({ run, task, api, act, busy, explain, onClose }) {
  const [usage, setUsage] = useState(null),
    [error, setError] = useState(""),
    [retry, setRetry] = useState(0);
  useEffect(() => {
    const request = new AbortController();
    let timer;
    async function load() {
      try {
        const data = await api(
          `/runs/${run.id}/milestones/${encodeURIComponent(task.id)}/usage`,
          undefined,
          request.signal,
        );
        if (!request.signal.aborted) {
          setUsage(data);
          setError("");
        }
      } catch (e) {
        if (!request.signal.aborted) setError(e.message);
      }
      if (task.state === "running" && !request.signal.aborted) timer = setTimeout(load, 3000);
    }
    load();
    return () => {
      request.abort();
      clearTimeout(timer);
    };
  }, [api, run.id, task.id, task.state, run.pricing_revision, retry]);
  return (
    <Dialog title={shortSummary(task.goal, 96) || task.id} onClose={onClose} className="work-detail-dialog">
      <div className="work-details">
        <div className="milestone-meta">
          <Status state={task.state} />
          <strong>{task.worker}</strong>
          <code>{task.id}</code>
        </div>
        <div className="assignment-facts">
          <MilestoneTime {...{ run, task, api }} />
          <dl>
            {task.assigned_at && (
              <div>
                <dt>Assigned</dt>
                <dd>
                  <time dateTime={task.assigned_at}>{new Date(task.assigned_at).toLocaleString()}</time>
                </dd>
              </div>
            )}
            {task.variant && (
              <div>
                <dt>Variant</dt>
                <dd>{task.variant}</dd>
              </div>
            )}
            <div>
              <dt>Tokens</dt>
              <dd>
                {usage?.tokens?.toLocaleString() ?? "—"}
                {usage?.partial_tokens && usage.tokens != null && " (partial)"}
              </dd>
            </div>
            <div>
              <dt>Est. cost</dt>
              <dd>
                {usage?.usd == null
                  ? "—"
                  : usage.usd.toLocaleString("en-US", {
                      style: "currency",
                      currency: "USD",
                      maximumFractionDigits: 6,
                    })}
                {usage?.partial_cost && usage.usd != null && " (partial)"}
              </dd>
            </div>
            <div>
              <dt>Worker turns</dt>
              <dd>{usage?.turns ?? "—"}</dd>
            </div>
          </dl>
          {error && (
            <p role="alert">
              {error}{" "}
              <Button variant="outline" onClick={() => setRetry((n) => n + 1)}>
                Retry usage
              </Button>
            </p>
          )}
        </div>
        {!!task.depends?.length && <p>Depends on {task.depends.join(", ")}</p>}
        {task.parent && <p>Delegated by {task.parent}</p>}
        <details className="assignment-description">
          <summary>Details</summary>
          <Markdown>{task.goal}</Markdown>
          {task.finding && (
            <>
              <h3>Result</h3>
              <Markdown>{task.finding}</Markdown>
            </>
          )}
          {!!task.notes?.length && (
            <>
              <h3>Notes</h3>
              {task.notes.map((n, i) => (
                <Markdown key={i}>{n}</Markdown>
              ))}
            </>
          )}
          {task.paths?.length > 0 && <p>Files · {task.paths.join(", ")}</p>}
          {task.commit && (
            <p>
              Commit <code>{task.commit}</code>
            </p>
          )}
        </details>
        {task.route && <RoutingEvidence route={task.route} />}
        <div className="actions">
          <Button
            variant="outline"
            onClick={() => {
              onClose();
              explain("milestone", task.id);
            }}
          >
            Explain this
          </Button>
          {canRetryMilestone(run, task) && (
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => act({ kind: "resume", task: task.id, tokens: retryTokens(run) })}
            >
              Retry milestone
            </Button>
          )}
        </div>
      </div>
    </Dialog>
  );
}

function AddTask({ run, act, busy, onClose }) {
  const [error, setError] = useState("");
  const [text, setText] = useState(""),
    [request, setRequest] = useState(null),
    [sending, setSending] = useState(false);
  const message = run.conversation?.find((m) => m.request_id === request && request);
  const reply = message && run.conversation.find((m) => m.reply_to === message.id);
  const waiting = request && (!message || ["running", "queued"].includes(message.state));
  async function create(e) {
    e.preventDefault();
    if (!text.trim() || sending) return;
    const id = crypto.randomUUID();
    setSending(true);
    setError("");
    if (await act({ kind: "propose-milestones", id, text })) setRequest(id);
    else setError("Couldn’t request milestones. Check the run status and try again.");
    setSending(false);
  }
  return (
    <Dialog title="Add task" onClose={onClose}>
      <form onSubmit={create} className="task-request">
        <Label htmlFor="task-request">What do you want to add?</Label>
        <Textarea
          id="task-request"
          rows={4}
          value={text}
          readOnly={!!waiting}
          onChange={(e) => setText(e.target.value)}
        />
        {!reply?.tasks?.length && (
          <Button
            type="submit"
            disabled={
              !text.trim() ||
              busy ||
              sending ||
              !!waiting ||
              run.conversation?.some((m) => ["running", "queued"].includes(m.state))
            }
          >
            Create milestone(s)
          </Button>
        )}
      </form>
      {waiting && <p role="status">Coordinator is preparing milestones…</p>}
      {error && <p role="alert">{error}</p>}
      {message?.error && <p role="alert">{message.error}</p>}
      {reply && (
        <>
          <Markdown>{reply.text}</Markdown>
          {reply.error && <p role="alert">{reply.error}</p>}
          {reply.tasks?.length > 0 && (
            <Proposal message={reply} {...{ run, act, busy }} onDecision={onClose} />
          )}
        </>
      )}
    </Dialog>
  );
}

const views = [
  ["table", "Table", List],
  ["map", "Map", Network],
  ["board", "Kanban", Columns3],
];
const lanes = [
  ["Queued", ["pending"]],
  ["In progress", ["running"]],
  ["Needs attention", ["blocked", "question", "interrupted"]],
  ["Done", ["committed", "abandoned"]],
];
export function Milestones({ run, api, act, busy, explain, question }) {
  const [view, setView] = useState("table"),
    [selected, setSelected] = useState(null),
    [adding, setAdding] = useState(false),
    [earlier, setEarlier] = useState(false);
  const all = run.tasks || [],
    offset = earlier ? 0 : run.task_start || 0;
  const tasks = all.slice(offset),
    task = all.find((t) => t.id === selected);
  const card = (t) => (
    <Button variant="ghost" data-task={t.id} onClick={() => setSelected(t.id)} aria-haspopup="dialog">
      <small aria-label={`Milestone ${all.indexOf(t) + 1}`}>{all.indexOf(t) + 1}</small>
      <strong>{shortSummary(t.goal, 90) || t.id}</strong>
    </Button>
  );
  function details(beforeExplain) {
    return (
      task && (
        <MilestoneDetails
          key={task.id}
          {...{ task, run, api, act, busy }}
          explain={(kind, id) => {
            beforeExplain?.();
            explain(kind, id);
          }}
          onClose={() => setSelected(null)}
        />
      )
    );
  }
  return (
    <section className="milestones-panel">
      <header className="work-section-heading">
        <h2>Milestones</h2>
        <Button
          onClick={() => setAdding(true)}
          disabled={
            run.state === "stopped" || (run.input.token_target > 0 && run.tokens >= run.input.token_target)
          }
        >
          <Plus />
          Add task
        </Button>
      </header>
      <div className="view-selector" aria-label="Milestone views">
        {views.map(([id, label, Icon]) => (
          <Button
            key={id}
            size="sm"
            variant={view === id ? "secondary" : "ghost"}
            aria-pressed={view === id}
            onClick={() => setView(id)}
          >
            <Icon />
            {label}
          </Button>
        ))}
        {run.task_start > 0 && (
          <Button size="sm" variant="outline" aria-pressed={earlier} onClick={() => setEarlier(!earlier)}>
            Earlier milestones
          </Button>
        )}
        {tasks.length > 0 && (
          <small>
            {tasks.filter((t) => t.state === "committed").length} of {tasks.length} complete
          </small>
        )}
      </div>
      {run.benchmark_note && (
        <details>
          <summary>Model routing</summary>
          <p>{run.benchmark_note}</p>
        </details>
      )}
      {view === "map" ? (
        <AssignmentMap {...{ run, tasks, offset }} onDetails={setSelected} renderDetails={details} />
      ) : !tasks.length ? (
        <p>Waiting for the coordinator’s plan.</p>
      ) : view === "table" ? (
        <Table aria-label="Milestones in order added">
          <TableHeader>
            <TableRow>
              {["Milestone", "Owner", "Status"].map((k) => (
                <TableHead key={k}>{k}</TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {tasks.map((t) => (
              <TableRow key={t.id}>
                <TableCell>
                  {card(t)}
                  {t.question && t.state === "question" && !run.project && question(t)}
                </TableCell>
                <TableCell>{t.worker}</TableCell>
                <TableCell>
                  <Status state={t.state} />
                  {canRetryMilestone(run, t) && (
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={busy}
                      onClick={() => act({ kind: "resume", task: t.id, tokens: retryTokens(run) })}
                    >
                      Retry milestone
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      ) : (
        <div className="kanban">
          {lanes.map(([label, states], i) => {
            const items = tasks.filter(
              (t) => states.includes(t.state) || (i === 0 && !lanes.some(([, ss]) => ss.includes(t.state))),
            );
            return (
              <section key={label}>
                <h3>
                  {label}
                  <small>{items.length}</small>
                </h3>
                {items.map((t) => (
                  <div className="board-card" key={t.id}>
                    {card(t)}
                    <footer>
                      {t.worker}
                      <Status state={t.state} />
                    </footer>
                  </div>
                ))}
              </section>
            );
          })}
        </div>
      )}
      {view !== "map" && details()}
      {adding && <AddTask {...{ run, act, busy }} onClose={() => setAdding(false)} />}
    </section>
  );
}
