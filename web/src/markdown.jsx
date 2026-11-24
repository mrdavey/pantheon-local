import React, { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import "./markdown.css";

const components = {
  a({ href, children }) {
    return /^(https?:\/\/|mailto:)/i.test(href || "")
      ? <a href={href} target="_blank" rel="noreferrer">{children}</a>
      : <span>{children}</span>;
  },
  // Agent output may reference private paths or remote tracking images.
  img({ alt }) { return alt ? <span>{alt}</span> : null; },
  table({ children }) { return <div className="markdown-table"><table>{children}</table></div>; },
};

export const Markdown = memo(function Markdown({ children, text, className = "" }) {
  return <div className={`markdown ${className}`.trim()}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>{children ?? text ?? ""}</ReactMarkdown>
  </div>;
});
