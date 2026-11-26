import React from "react";
import { ChevronRight } from "lucide-react";
import { Button } from "./components/ui/button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "./components/ui/collapsible";

export function Disclosure({ title, children, ...props }) {
  return (
    <Collapsible {...props}>
      <CollapsibleTrigger asChild>
        <Button type="button" variant="ghost" className="disclosure-trigger">
          <ChevronRight aria-hidden="true" />
          {title}
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>{children}</CollapsibleContent>
    </Collapsible>
  );
}
