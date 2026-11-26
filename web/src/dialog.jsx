import React, { useLayoutEffect, useRef } from "react";
import { Dialog as Root, DialogContent, DialogTitle } from "./components/ui/dialog";
import { Button } from "./components/ui/button";
import { MessageCircleQuestion, X } from "lucide-react";
import "./dialog.css";

export function Dialog({ title, onClose, children, className = "", placement = "center", pageKey, titleAction, onExplain }) {
  const opener = useRef(document.activeElement);
  const content = useRef(null);
  const previousPage = useRef(pageKey);
  const heading = useRef(null);
  useLayoutEffect(() => {
    if (previousPage.current !== pageKey) heading.current?.focus();
    previousPage.current = pageKey;
  }, [pageKey]);
  // A child dialog can receive Escape before Radix has registered its dismiss layer.
  useLayoutEffect(() => {
    function escape(event) {
      if (event.key !== "Escape" || event.isComposing || event.defaultPrevented) return;
      if (event.target.matches?.('input[role="combobox"][aria-expanded="true"]')) return;
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
    <DialogContent placement={placement} ref={content} showCloseButton={false} aria-describedby={undefined} className={`workbench-dialog ${className}`}
      onEscapeKeyDown={event => {
        if (event.target.matches?.('input[role="combobox"][aria-expanded="true"]')) event.preventDefault();
      }}
      onCloseAutoFocus={event => {
        event.preventDefault();
        // Radix restores focus asynchronously. Keep a newer user-selected field active.
        const focused = document.activeElement;
        if (opener.current?.isConnected && (!focused || focused === document.body || focused === opener.current)) {
          opener.current.focus({ preventScroll: true });
        }
      }}>
      <header className="dialog-heading"><DialogTitle ref={heading} tabIndex={-1}>{title}</DialogTitle>{titleAction}
        <div className="dialog-actions">{onExplain && <Button variant="ghost" size="sm" onClick={() => {onClose(); onExplain();}}><MessageCircleQuestion aria-hidden="true"/>Explain this</Button>}
        <Button variant="ghost" size="icon" type="button" aria-label="Close dialog" onClick={onClose}><X /></Button></div>
      </header>
      {children}
    </DialogContent>
  </Root>;
}
