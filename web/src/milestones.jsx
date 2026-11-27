import { changeRoute } from "./navigation";
import { Disclosure } from "./disclosure";
import { ActivityLog } from "./activity-log";
import React, { useEffect, useState, useId } from "react";
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
import { Status, pendingConversation, preparingMilestones } from "./status";
import { canRetryMilestone, retryTokens, shortSummary } from "./run-summary";
import { RoutingEvidence } from "./conversation";
import { AssignmentMap } from "./assignment-map";

export function MilestoneDetails({ run, task, api, act, busy, explain, openMilestone, onClose, route }) {
  const [expandedId, setExpandedId] = useState(null);
  const detailsOpen = expandedId === task.id;
  const detailsId = useId();
  return (
    <Dialog title={shortSummary(task.goal, 96) || task.id} onExplain={() => explain("milestone", task.id)} onClose={onClose} pageKey={task.id} placement="right"
      titleAction={<Button variant="ghost" size="sm" aria-expanded={detailsOpen} aria-controls={detailsId}
        onClick={() => setExpandedId(detailsOpen ? null : task.id)}>Details</Button>} className="work-detail-dialog">
      <div id={detailsId} hidden={!detailsOpen}>
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
      </div>
      <AssignmentFacts key={task.id} {...{ run, task, api, act, busy, openMilestone }} />
      <section aria-label="Milestone activity"><h3>Activity log</h3>
        <ActivityLog run={run} api={api} route={route} milestone={task.id} explain={(kind,id) => {onClose(); explain(kind,id);}}/>
      </section>
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
      <MilestoneLinks label="Superseded by" ids={task.superseded_by ? [task.superseded_by] : []} tasks={run.tasks} open={openMilestone} />
      <MilestoneLinks ids={task.depends} tasks={run.tasks} open={openMilestone} />
      <MilestoneLinks
        label="Delegated by"
        ids={task.parent ? [task.parent] : []}
        tasks={run.tasks}
        open={openMilestone}
      />
      {(task.finding || task.notes?.length > 0) && <section><h3>Latest update</h3><Markdown>{task.finding || task.notes.at(-1)}</Markdown></section>}

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

function AddTask({ run, act, busy, request, onClose, onReply }) {
  const [error, setError] = useState("");
  const [draft, setDraft] = useState(null);
  const [sending, setSending] = useState(false);
  const message = request && run.conversation?.find(m => m.request_id === request);
  const reply = message && run.conversation.find(m => m.reply_to === message.id);
  const replyId = reply?.id;
  const waiting = message?.state === "running" || message?.state === "queued";
  const text = draft ?? message?.text ?? "";
  const canSubmit = text.trim() && !busy && !sending && !waiting && !reply && !pendingConversation(run);
  const problem = error || (!sending && message?.error);

  useEffect(() => {
    if (replyId) onReply(replyId);
  }, [replyId, onReply]);

  async function create(event) {
    event.preventDefault();
    if (!canSubmit) return;
    const id = crypto.randomUUID();
    const origin = location.search;
    setSending(true);
    setError("");
    const ok = await act({ kind: "propose-milestones", id, text });
    setSending(false);
    // The request can finish after the user has dismissed the dialog.
    if (location.search !== origin) return;
    if (ok) changeRoute({proposalRequest: id}, {replace:true});
    else setError("Couldn’t request milestones. Check the run status and try again.");
  }

  return (
    <Dialog title="Add task" onClose={onClose}>
      <form onSubmit={create} className="task-request">
        <Label htmlFor="task-request">What do you want to add?</Label>
        <Textarea
          id="task-request"
          rows={4}
          value={text}
          readOnly={sending || waiting}
          onChange={(e) => setDraft(e.target.value)}
        />
        <Button type="submit" disabled={!canSubmit}>Create milestone(s)</Button>
      </form>
      {(sending || waiting) && <p role="status">{preparingMilestones}</p>}
      {problem && <p role="alert">{problem}</p>}
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
  ["Done", ["committed", "abandoned", "superseded"]],
];
export function Milestones({ route, run, api, act, busy, explain, question, selected, setSelected, openProposalReply }) {
  const view = views.some(([id]) => id === route.layout) ? route.layout : "table";
  const setView = layout => changeRoute({tab:"milestones", layout, milestone:null});
  const adding = route.tab === "milestones" && route.add === "task";
  const setAdding = open => changeRoute({tab:"milestones", add:open ? "task" : null, proposalRequest:null});
  const earlier = route.earlier === "yes";
  const setEarlier = open => changeRoute({tab:"milestones", earlier:open ? "yes" : null});
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
          {...{ task, run, api, act, busy, route }}
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
            {tasks.filter((t) => t.state === "committed").length} of {tasks.filter(t => !["abandoned", "superseded"].includes(t.state)).length} complete
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
                  <Card asChild key={t.id}>
                    <button className="board-card" type="button" data-task={t.id} aria-haspopup="dialog" onClick={() => setSelected(t.id)}>
                      <small>Milestone {all.indexOf(t) + 1}</small>
                      <strong>{shortSummary(t.goal, 90) || t.id}</strong>
                      <span className="board-owner">{t.worker}<Status state={t.state}/></span>
                    </button>
                  </Card>
                ))}
              </section>
            );
          })}
        </div>
      )}
      {view !== "map" && details()}
      {adding && (
        <AddTask {...{ run, act, busy }} request={route.proposalRequest} onReply={openProposalReply} onClose={() => setAdding(false)} />
      )}
    </section>
  );
}
