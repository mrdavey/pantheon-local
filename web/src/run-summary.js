// Older runs and runs awaiting their first coordinator turn only have a brief.
export function runTitle(run) {
  const goal = (run.input?.prompt || run.brief || "").split(/\r?\n/).map((line) => line.trim()
    .replace(/^#+\s*/, "").replace(/^(?:goal|objective|task|brief)\s*(?::\s*|$)/i, ""))
    .find(Boolean);
  const title = (run.title || goal || "Untitled run").replace(/\s+/g, " ").trim();
  const chars = Array.from(title);
  if (chars.length <= 80) return title;
  const prefix = chars.slice(0, 79).join("");
  return (prefix.replace(/\s+\S*$/, "") || prefix) + "…";
}

export function blockedTasks(run) {
  return run.state === "working" || run.state === "blocked"
    ? (run.tasks || []).slice(run.task_start || 0).filter((task) => task.state === "blocked") : [];
}
