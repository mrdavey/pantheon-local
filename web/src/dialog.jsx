import React, { useLayoutEffect, useRef } from "react";
import { Dialog as Root, DialogContent, DialogTitle } from "./components/ui/dialog";
import { Button } from "./components/ui/button";
import { X } from "lucide-react";
import "./dialog.css";

export function Dialog({ title, onClose, children, className = "" }) {
  const opener = useRef(document.activeElement);
  const content = useRef(null);
  // A child dialog can receive Escape before Radix has registered its dismiss layer.
  useLayoutEffect(() => {
    function escape(event) {
      if (event.key !== "Escape" || event.isComposing || event.defaultPrevented) return;
      const dialogs = document.querySelectorAll('[data-slot="dialog-content"][data-state="open"]');
      if (dialogs[dialogs.length - 1] !== content.current) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (!event.repeat) onClose();
    }
    window.addEventListener("keydown", escape, true);
    return () => window.removeEventListener("keydown", escape, true);
  }, [onClose]);
  return <Root open onOpenChange={open => { if (!open) onClose(); }}>
    <DialogContent ref={content} showCloseButton={false} aria-describedby={undefined} className={`workbench-dialog ${className}`}
      onCloseAutoFocus={event => {
        event.preventDefault();
        // Radix restores focus asynchronously. Keep a newer user-selected field active.
        const focused = document.activeElement;
        if (opener.current?.isConnected && (!focused || focused === document.body || focused === opener.current)) {
          opener.current.focus({ preventScroll: true });
        }
      }}>
      <header className="dialog-heading"><DialogTitle>{title}</DialogTitle>
        <Button variant="ghost" size="icon" type="button" aria-label="Close dialog" onClick={onClose}><X /></Button>
      </header>
      {children}
    </DialogContent>
  </Root>;
}
