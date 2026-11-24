import { fromMarkdown } from "mdast-util-from-markdown";
import { toString } from "mdast-util-to-string";

// Buttons and headings need only the visible words, including inline code and link labels.
function plainText(value) {
  return /[`*_~\[<>#!]/.test(value)
    ? toString(fromMarkdown(value), { includeHtml: false }).trim()
    : value;
}

// Older runs and runs awaiting their first coordinator turn only have a brief.
export function runTitle(run) {
  const goal = (run.input?.prompt || run.brief || "").split(/\r?\n/).map((line) => line.trim()
    .replace(/^#+\s*/, "").replace(/^(?:goal|objective|task|brief)\s*(?::\s*|$)/i, ""))
    .find(Boolean);
  const title = (plainText(run.title || goal || "") || "Untitled run").replace(/\s+/g, " ").trim();
  const chars = Array.from(title);
  if (chars.length <= 80) return title;
  const prefix = chars.slice(0, 79).join("");
  return (prefix.replace(/\s+\S*$/, "") || prefix) + "…";
}

export function blockedTasks(run) {
  return run.state === "working" || run.state === "blocked"
    ? (run.tasks || []).slice(run.task_start || 0).filter((task) => task.state === "blocked") : [];
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
