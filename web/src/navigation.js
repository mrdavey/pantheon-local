import { useMemo, useSyncExternalStore } from "react";

const routeEvent = "workbench-navigation";

function subscribe(notify) {
  window.addEventListener("popstate", notify);
  window.addEventListener(routeEvent, notify);
  return () => {
    window.removeEventListener("popstate", notify);
    window.removeEventListener(routeEvent, notify);
  };
}

export function useRoute() {
  const search = useSyncExternalStore(subscribe, () => location.search);
  return useMemo(() => Object.fromEntries(new URLSearchParams(search)), [search]);
}

export function changeRoute(values, { reset = false, replace = false } = {}) {
  const url = new URL(location.href);
  if (reset) url.search = "";
  for (const [key, value] of Object.entries(values)) {
    if (value == null || value === "") url.searchParams.delete(key);
    else url.searchParams.set(key, value);
  }
  if (url.search === location.search) return;
  history[replace ? "replaceState" : "pushState"](null, "", url.pathname + url.search);
  window.dispatchEvent(new Event(routeEvent));
}

export function destination(selection) {
  if (selection === "") return { page: "new" };
  if (["setup", "inbox", "runs"].includes(selection)) return { page: selection };
  return { page: "run", run: selection };
}
