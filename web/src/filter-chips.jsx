import React from "react";
import { X } from "lucide-react";
import { Button } from "./components/ui/button";

export function FilterChips({ filters, onRemove }) {
  const active = filters.filter(([, , value]) => value);
  if (!active.length) return null;

  return (
    <div className="filter-chips" role="group" aria-label="Selected filters">
      {active.map(([key, label, value]) => (
        <Button
          key={key}
          size="sm"
          variant="secondary"
          aria-label={`Remove ${label.toLowerCase()} filter ${value}`}
          onClick={() => onRemove(key)}
        >
          <span>{label}</span>
          {value}
          <X aria-hidden="true" />
        </Button>
      ))}
    </div>
  );
}
