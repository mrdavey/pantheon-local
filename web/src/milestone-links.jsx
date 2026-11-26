import React from "react";
import { ArrowUpRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { shortSummary } from "./run-summary";

export function MilestoneLinks({ ids, tasks, open, label = "Depends on" }) {
  if (!ids?.length) return null;
  return (
    <div className="milestone-links">
      <span>{label}</span>
      {ids.map((id) => {
        const target = tasks.find((t) => t.id === id);
        return target ? (
          <Button
            key={id}
            variant="secondary"
            size="sm"
            title={id}
            aria-label={"Open milestone " + id}
            onClick={() => open(id)}
          >
            {shortSummary(target.goal, 52) || id}
            <ArrowUpRight />
          </Button>
        ) : (
          <code key={id}>{id}</code>
        );
      })}
    </div>
  );
}
