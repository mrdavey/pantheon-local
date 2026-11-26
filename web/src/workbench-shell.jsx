import React, { useLayoutEffect, useState } from "react";
import { Activity, Box, Cpu, Landmark, Monitor } from "lucide-react";
import { NativeSelect, NativeSelectOption } from "./components/ui/native-select";
import { statusLabel } from "./status";

export function WorkbenchHeader({ project, connected, ready }) {
  const [appearance, setAppearance] = useState(() => localStorage.getItem("pantheon-appearance") || "dark");
  useLayoutEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    function apply() {
      const resolved = appearance === "system" ? (media.matches ? "dark" : "light") : appearance;
      document.documentElement.dataset.appearance = resolved;
      document.documentElement.classList.toggle("dark", resolved === "dark");
    }
    apply();
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [appearance]);
  return (
    <header className="workbench-header">
      <div className="brand">
        <Landmark aria-hidden="true" />
        Pantheon Local
      </div>
      {project && (
        <span className="header-project" title={project}>
          {project.split("/").filter(Boolean).at(-1) || project}
        </span>
      )}
      <div className="header-actions">
        <span className="local-indicator" data-online={connected}>
          <i />
          {connected ? "Local" : ready ? "Reconnecting" : "Connecting"}
        </span>
        <div className="appearance">
          <Monitor aria-hidden="true" />
          <NativeSelect
            aria-label="Appearance"
            value={appearance}
            onChange={(e) => {
              localStorage.setItem("pantheon-appearance", e.target.value);
              setAppearance(e.target.value);
            }}
          >
            {["dark", "light", "system"].map((v) => (
              <NativeSelectOption key={v} value={v}>
                {v[0].toUpperCase() + v.slice(1)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
        </div>
      </div>
    </header>
  );
}

export function RunFooter({ run }) {
  return (
    <footer className="run-strip">
      <span>
        <Activity />
        Run {run.id.slice(0, 6)}
        <i /> {statusLabel(run.state)}
      </span>
      <span>
        <Cpu />
        {run.input.workers.length} {run.input.workers.length === 1 ? "worker" : "workers"}
      </span>
      <span>
        <Box />
        Settled tokens {run.tokens?.toLocaleString() ?? "—"}
      </span>
    </footer>
  );
}
