import React, { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Table, TableBody, TableHead, TableHeader, TableRow, TableCell } from "./components/ui/table";
import "./markdown.css";

const components = {
  a({ href, children }) {
    return /^(https?:\/\/|mailto:)/i.test(href || "")
      ? <a href={href} target="_blank" rel="noreferrer">{children}</a>
      : <span>{children}</span>;
  },
  // Agent output may reference private paths or remote tracking images.
  img({ alt }) { return alt ? <span>{alt}</span> : null; },
  thead: ({ children }) => <TableHeader>{children}</TableHeader>,
  tbody: ({ children }) => <TableBody>{children}</TableBody>,
  tr: ({ children }) => <TableRow>{children}</TableRow>,
  th: ({ children, style }) => <TableHead style={style}>{children}</TableHead>,
  td: ({ children, style }) => <TableCell style={style}>{children}</TableCell>,
  table({ children }) { return <Table className="markdown-table">{children}</Table>; },
};

export const Markdown = memo(function Markdown({ children, text, className = "" }) {
  return <div className={`markdown ${className}`.trim()}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>{children ?? text ?? ""}</ReactMarkdown>
  </div>;
});
