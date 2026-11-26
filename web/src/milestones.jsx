import { Disclosure } from "./disclosure";
import React, { useEffect, useState } from "react";
import { Columns3, List, Network, Plus } from "lucide-react";
import { Button } from "./components/ui/button";
import { MilestoneTable } from "./milestone-table";
import { MilestoneLinks } from "./milestone-links";
import { Card } from "./components/ui/card";
import { Textarea } from "./components/ui/textarea";
import { Label } from "./components/ui/label";
import { Dialog } from "./dialog";
import { Markdown } from "./markdown";
import { MilestoneTime } from "./milestone-time";
import { Status } from "./status";
import { canRetryMilestone, retryTokens, shortSummary } from "./run-summary";
import { Proposal, RoutingEvidence } from "./conversation";
import { AssignmentMap } from "./assignment-map";

export function MilestoneDetails({ run, task, api, act, busy, explain, openMilestone, onClose }) {
  return (
    <Dialog title={shortSummary(task.goal, 96) || task.id} onExplain={() => explain("milestone", task.id)} onClose={onClose} pageKey={task.id} placement="right" className="work-detail-dialog">
      <AssignmentFacts key={task.id} {...{ run, task, api, act, busy, openMilestone }} />
    </Dialog>
  );
}

function AssignmentFacts({ run, task, api, act, busy, openMilestone }) {
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
    <div className="work-details">
      <div className="milestone-meta">
        <Status state={task.state} />
        <span>Milestone {run.tasks.findIndex(t => t.id === task.id) + 1}</span>
        <code>{task.id}</code>
      </div>
      <div className="assignment-facts">
        <MilestoneTime {...{ run, task, api }} />
        <dl>
          <div><dt>Worker</dt><dd>{task.worker}</dd></div>
          {task.paths?.length > 0 && <div><dt>Scope</dt><dd>{task.paths.join(", ")}</dd></div>}
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
      <MilestoneLinks ids={task.depends} tasks={run.tasks} open={openMilestone} />
      <MilestoneLinks
        label="Delegated by"
        ids={task.parent ? [task.parent] : []}
        tasks={run.tasks}
        open={openMilestone}
      />
      {(task.finding || task.notes?.length > 0) && <section><h3>Latest update</h3><Markdown>{task.finding || task.notes.at(-1)}</Markdown></section>}
      <Disclosure className="assignment-description" title="Details">
        <Markdown>{task.goal}</Markdown>
        {!!task.notes?.length && (
          <>
            <h3>Notes</h3>
            {task.notes.map((n, i) => (
              <Markdown key={i}>{n}</Markdown>
            ))}
          </>
        )}
        {task.commit && (
          <p>
            Commit <code>{task.commit}</code>
          </p>
        )}
      </Disclosure>
      {task.route && <RoutingEvidence route={task.route} />}
      <div className="actions">
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
  );
}

function AddTask({ run, act, busy, onClose, openMilestone }) {
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
            <Proposal
              message={reply}
              {...{ run, act, busy }}
              onDecision={onClose}
              openMilestone={(id) => {
                onClose();
                openMilestone(id);
              }}
            />
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
export function Milestones({ run, api, act, busy, explain, question, selected, setSelected }) {
  const [view, setView] = useState("table"),
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
          {...{ task, run, api, act, busy }}
          openMilestone={setSelected}
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
          <Button size="sm" className="history-toggle" variant="outline" aria-pressed={earlier} onClick={() => setEarlier(!earlier)}>
            Earlier milestones
          </Button>
        )}
        {tasks.length > 0 && (
          <small>
            {tasks.filter((t) => t.state === "committed").length} of {tasks.length} complete
          </small>
        )}
        <Button className="add-task"
          onClick={() => setAdding(true)}
          disabled={
            run.state === "stopped" || (run.input.token_target > 0 && run.tokens >= run.input.token_target)
          }
        >
          <Plus />
          Add task
        </Button>
      </div>
      {run.benchmark_note && (
        <Disclosure title="Model routing">
          <p>{run.benchmark_note}</p>
        </Disclosure>
      )}
      {view === "map" ? (
        <AssignmentMap
          {...{ run, tasks, offset }}
          onDetails={setSelected}
          renderDetails={details}
        />
      ) : !tasks.length ? (
        <p>Waiting for the coordinator’s plan.</p>
      ) : view === "table" ? (
        <MilestoneTable {...{ run, tasks, api, act, busy, card, question }} onSelect={setSelected} />
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
                  <Card
                    className="board-card"
                    key={t.id}
                    onClick={(e) => {
                      if (!e.target.closest("button")) {
                        e.currentTarget.querySelector("button").focus();
                        setSelected(t.id);
                      }
                    }}
                  >
                    {card(t)}
                    <footer>
                      {t.worker}
                      <Status state={t.state} />
                    </footer>
                  </Card>
                ))}
              </section>
            );
          })}
        </div>
      )}
      {view !== "map" && details()}
      {adding && (
        <AddTask {...{ run, act, busy }} openMilestone={setSelected} onClose={() => setAdding(false)} />
      )}
    </section>
  );
}
