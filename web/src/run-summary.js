import { fromMarkdown } from "mdast-util-from-markdown";
import { toString } from "mdast-util-to-string";

// Buttons and headings need only the visible words, including inline code and link labels.
function plainText(value) {
  return /[`*_~\[<>#!]/.test(value)
    ? toString(fromMarkdown(value), { includeHtml: false }).trim()
    : value;
}

// Older runs and runs awaiting their first coordinator turn only have a brief.
export function runTitle(run, { truncate = true } = {}) {
  const goal = (run.input?.prompt || run.brief || "").split(/\r?\n/).map((line) => line.trim()
    .replace(/^#+\s*/, "").replace(/^(?:goal|objective|task|brief)\s*(?::\s*|$)/i, ""))
    .find(Boolean);
  const title = (plainText(run.title || goal || "") || "Untitled run").replace(/\s+/g, " ").trim();
  if (!truncate) return title;
  const chars = Array.from(title);
  if (chars.length <= 80) return title;
  const prefix = chars.slice(0, 79).join("");
  return (prefix.replace(/\s+\S*$/, "") || prefix) + "…";
}

export function runProjectPath(run) {
  const { Root, Subdir } = run.repo || {};
  if (!Root) return run.input?.path || "";
  return Subdir && Subdir !== "." ? Root.replace(/\/$/, "") + "/" + Subdir : Root;
}

export function runInitials(title) {
  const words = title.match(/[\p{L}\p{N}]+/gu) || [title];
  return words.slice(0, 2).map(word => Array.from(word)[0]).join("").toUpperCase();
}

export function blockedTasks(run) {
  if (run.state === "working" || run.state === "blocked") {
    return (run.tasks || []).slice(run.task_start || 0).filter((task) => task.state === "blocked");
  }
  if (run.state === "evaluating" || run.state === "review") {
    return (run.tasks || []).slice(run.task_start || 0).filter((task) => task.state === "blocked" && task.conflict);
  }
  return [];
}

export function overTokenTarget(run) {
  return run.state === "budget" || ((run.input?.token_target || 0) > 0 && (run.tokens || 0) >= run.input.token_target);
}

export function canRetryMilestone(run, task) {
  if (!["blocked", "interrupted"].includes(task.state)) return false;
  if (run.state === "stopped" && (run.turns || []).some((t) => t.state === "running")) return false;
  if (["stopped", "blocked", "budget", "working"].includes(run.state)) return true;
  return Boolean(task.conflict) && ["evaluating", "review"].includes(run.state);
}

export function retryTokens(run) {
  return overTokenTarget(run) ? 0 : run.input.token_target;
}

export function shortSummary(value, limit = 120) {
  const line = String(value || "").split(/\r?\n/).map((s) => s.trim().replace(/^#{1,6}\s+/, "")).find(Boolean) || "";
  const text = plainText(line);
  const sentence = text.match(/^.*?[.!?](?=\s|$)/)?.[0] || text;
  const chars = Array.from(sentence);
  if (chars.length <= limit) return sentence;
  const prefix = chars.slice(0, limit - 1).join("");
  return (prefix.replace(/\s+\S*$/, "") || prefix) + "…";
}
