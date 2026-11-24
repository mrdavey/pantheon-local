import React, { useLayoutEffect, useId, useRef } from "react";
import "./dialog.css";

export function Dialog({ title, onClose, children, className = "" }) {
  const ref = useRef(null);
  const heading = useId();
  useLayoutEffect(() => {
    const dialog = ref.current;
    const opener = document.activeElement;
    dialog.showModal();
    return () => {
      dialog.close();
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);
  return <dialog ref={ref} aria-labelledby={heading} className={`workbench-dialog ${className}`}
    onCancel={(event) => { event.preventDefault(); onClose(); }}>
    <header className="dialog-heading">
      <h2 id={heading}>{title}</h2>
      <button type="button" className="dialog-close" aria-label="Close dialog" onClick={onClose}>×</button>
    </header>
    {children}
  </dialog>;
}
