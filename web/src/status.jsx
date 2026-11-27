import React, { createContext, useCallback, useContext, useLayoutEffect, useState } from "react";
import { Landmark } from "lucide-react";
import { Badge } from "./components/ui/badge";
import { Spinner } from "./components/ui/spinner";
import { Progress } from "./components/ui/progress";
import { blockedTasks } from "./run-summary.js";

const labels = {
  planning: "Planning", question: "Needs an answer", checks: "Review checks", working: "In progress",
  evaluating: "Checking candidates", repairing: "Routing a repair", review: "Ready to review", landed: "Landed",
  budget: "Token target reached", blocked: "Blocked", stopped: "Stopped", pending: "Queued",
  running: "Working", committed: "Committed", interrupted: "Interrupted", passed: "Passed",
  failed: "Failed", conflict: "Merge conflict", inconclusive: "Inconclusive", cancelled: "Canceled",
  superseded: "Superseded", abandoned: "Abandoned", stale: "Stale", queued: "Queued", received: "Finished",
  "milestone-blocked": "Milestone blocked", "awaiting-review": "Awaiting coordinator",
  "needs-attention": "Needs attention", reviewed: "Reviewed", recorded: "Recorded",
};

const PendingOperations = createContext(null);

function LoadingMark() {
  return <Landmark className="loading-mark" aria-hidden="true" />;
}

export function LoadingProvider({ children }) {
  const [operations, setOperations] = useState([]);
  const register = useCallback(message => {
    const entry = { message };
    setOperations(list => [...list, entry]);
    return () => setOperations(list => list.filter(item => item !== entry));
  }, []);
  const message = operations.at(-1)?.message;
  return <PendingOperations.Provider value={register}>
    <div className="loading-status window-status" data-loading={!!message}>
      {message && <LoadingMark />}
      <span role="status">{message}</span>
      {message && <Progress value={null} aria-label={message} />}
    </div>
    {children}
  </PendingOperations.Provider>;
}

export function Loading({ children, top = false }) {
  const register = useContext(PendingOperations);
  useLayoutEffect(() => register(children), [children, register]);
  return top ? null : <span className="loading-status"><LoadingMark /><span>{children}</span></span>;
}

export const preparingMilestones = "Coordinator is preparing milestones…";
export const waitingForCoordinator = "Waiting for the coordinator…";
export const pendingConversation = run => run.conversation?.find(({ state }) => state === "queued" || state === "running");

export function conversationLoadingMessage(run) {
  if (run.state === "stopped") return "";
  const pending = pendingConversation(run);
  if (!pending) return "";
  return pending.intent === "add" ? preparingMilestones : waitingForCoordinator;
}

const coordinatorActivities = {
  pricing: "Finding model prices", plan: "Creating the plan", observe: "Reviewing worker progress",
  repair: "Planning a repair", chat: "Responding to your message",
};
const coordinatorTurn = run => run.turns?.findLast(t =>
  t.state === "running" && coordinatorActivities[t.role] && (run.state !== "stopped" || t.role === "pricing"));

export function runLoadingMessage(run) {
  const reply = conversationLoadingMessage(run);
  if (reply) return reply;
  const active = coordinatorTurn(run);
  if (active) return coordinatorActivities[active.role] + "…";
  if (run.state === "stopped") return "";
  if (run.combination) return "Planning the combination…";
  return ({
    planning: "Creating the plan…",
    repairing: "Planning a repair…",
    evaluating: "Checking the implementation…",
  })[run.state] || "";
}

export const statusLabel = state => Object.hasOwn(labels, state) ? labels[state] : state;

export function runStatus(run) {
  return run.state === "working" && blockedTasks(run).length ? "milestone-blocked" : run.state;
}

export function Status({ state, run, compact = false }) {
  const status = run ? runStatus(run) : state;
  const active = run ? run.turns?.some(t => t.state === "running") : state === "running";
  return <Badge variant="outline" className={"state " + (status === "milestone-blocked" ? "blocked" : status)}>
    {active && <Spinner aria-hidden="true" />}
    {compact ? <span className="visually-hidden">{statusLabel(status)}</span> : statusLabel(status)}
  </Badge>;
}

export function coordinatorStatus(run) {
  const current = coordinatorTurn(run);
  if (current) return coordinatorActivities[current.role];
  if (run.observer_error) return "Coordinator review needs attention";
  const workers = run.tasks?.filter(t => t.state === "running").length || 0;
  if (run.state === "working" && workers) return `Following ${workers} active milestone${workers === 1 ? "" : "s"}`;
  return ({ planning: "Preparing the plan", working: "Checking milestone dependencies", checks: "Waiting for check approval",
    question: "Waiting for your answer", stopped: "Run paused", evaluating: "Checking the implementation",
    review: "Ready for your review", landed: "Implementation landed" })[run.state] || labels[run.state] || run.state;
}
