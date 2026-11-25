import React, { useRef } from "react";
import { Dialog as Root, DialogContent, DialogTitle } from "./components/ui/dialog";
import { Button } from "./components/ui/button";
import { X } from "lucide-react";
import "./dialog.css";

export function Dialog({ title, onClose, children, className = "" }) {
  const opener = useRef(document.activeElement);
  return <Root open onOpenChange={open => { if (!open) onClose(); }}>
    <DialogContent showCloseButton={false} aria-describedby={undefined} className={`workbench-dialog ${className}`}
      onCloseAutoFocus={event => {
        event.preventDefault();
        if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
      }}>
      <header className="dialog-heading"><DialogTitle>{title}</DialogTitle>
        <Button variant="ghost" size="icon" type="button" aria-label="Close dialog" onClick={onClose}><X /></Button>
      </header>
      {children}
    </DialogContent>
  </Root>;
}
