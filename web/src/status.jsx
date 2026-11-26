import React, { createContext, useCallback, useContext, useEffect, useState } from "react";
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
};

const PendingOperations = createContext(null);

export function LoadingProvider({ children }) {
  const [operations, setOperations] = useState([]);
  const register = useCallback(message => {
    const entry = { message };
    setOperations(list => [...list, entry]);
    return () => setOperations(list => list.filter(item => item !== entry));
  }, []);
  const message = operations.at(-1)?.message;
  return <PendingOperations.Provider value={register}>
    {children}
    {message && <div className="loading-status window-status" role="status"><Spinner aria-hidden="true" />
      <span>{message}</span><Progress value={null} aria-label={message} />
    </div>}
  </PendingOperations.Provider>;
}

export function Loading({ children, top = false }) {
  const register = useContext(PendingOperations);
  useEffect(() => register(children), [children, register]);
  return top ? null : <div className="loading-status"><Spinner aria-hidden="true" /><span>{children}</span></div>;
}

export function Status({ state, run }) {
  const blocked = run && state === "working" && blockedTasks(run).length > 0;
  const active = run ? run.turns?.some(t => t.state === "running") : state === "running";
  return <Badge variant="outline" className={"state " + (blocked ? "blocked" : state)}>
    {active && <Spinner aria-hidden="true" />}{blocked ? "Milestone blocked" : labels[state] || state}
  </Badge>;
}

export function coordinatorStatus(run) {
  const activities = { pricing: "Finding model prices", plan: "Creating the plan", observe: "Reviewing worker progress", repair: "Planning a repair", chat: "Responding to your message" };
  const current = run.turns?.findLast(t => t.state === "running" && activities[t.role]);
  if (current && run.state !== "stopped") return activities[current.role];
  if (run.observer_error) return "Coordinator review needs attention";
  const workers = run.tasks?.filter(t => t.state === "running").length || 0;
  if (run.state === "working" && workers) return `Following ${workers} active milestone${workers === 1 ? "" : "s"}`;
  return ({ planning: "Preparing the plan", working: "Checking milestone dependencies", checks: "Waiting for check approval",
    question: "Waiting for your answer", stopped: "Run paused", evaluating: "Checking the implementation",
    review: "Ready for your review", landed: "Implementation landed" })[run.state] || labels[run.state] || run.state;
}
