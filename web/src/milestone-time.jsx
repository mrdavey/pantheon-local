import React, { useEffect, useState } from "react";
import { Clock3 } from "lucide-react";
import { Button } from "./components/ui/button";
import { formatDuration } from "./duration";

export function MilestoneTime({ api, run, task }) {
  const [timing, setTiming] = useState(null);
  const [now, setNow] = useState(Date.now);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const last = run.turns?.findLast(t => t.role === "worker" && t.task === task.id);
  useEffect(() => {
    const request = new AbortController();
    setError("");
    api(`/runs/${run.id}/milestones/${encodeURIComponent(task.id)}/timing`, undefined, request.signal)
      .then(value => {
        if (!request.signal.aborted) { setTiming(value); setNow(Date.now()); }
      }).catch(err => { if (!request.signal.aborted) setError(err.message); });
    return () => request.abort();
  }, [api, run.id, run.state, task.id, task.state, last?.id, last?.state, retry]);
  const live = timing?.running && task.state === "running" && run.state !== "stopped";
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live]);
  const end = live ? now : timing?.ended;
  const measured = timing?.started != null && end != null;
  const label = task.state === "committed" ? "Completed in" : "Elapsed time";
  const empty = task.state === "pending" && !last ? "Queued" : "Timing unavailable";
  return <div className="milestone-time">
    <Clock3 aria-hidden="true" /><strong>{label}</strong>
    {error ? <><span>{error}</span><Button size="sm" variant="outline" onClick={() => setRetry(n => n + 1)}>Retry timing</Button></>
      : measured ? <time aria-label={label} dateTime={`PT${Math.max(0, Math.floor((end - timing.started) / 1000))}S`}
        title={`Started ${new Date(timing.started).toLocaleString()}`}>{formatDuration(end - timing.started)}</time>
      : <span>{timing ? empty : "Loading…"}</span>}
  </div>;
}
