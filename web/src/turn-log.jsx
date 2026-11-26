import { Disclosure } from "./disclosure";
import React, { useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, ChevronsLeft, Pause, Play } from "lucide-react";
import { Button } from "./components/ui/button";
import { Loading } from "./status";
import { LogBody, messageRows } from "./evidence";
import "./turn-log.css";

const absoluteTime = new Intl.DateTimeFormat(undefined, {
  year: "numeric", month: "short", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit",
});
const relativeTime = new Intl.RelativeTimeFormat(undefined, { numeric: "always" });
export function LogTime({ at, now }) {
  const date = new Date(at);
  if (at == null || !Number.isFinite(date.getTime())) return <small>Time unavailable</small>;
  const seconds = Math.max(0, Math.floor((now - date.getTime()) / 1000));
  const [unit, divisor] = seconds < 60 ? ["second", 1] : seconds < 3600 ? ["minute", 60] :
    seconds < 86400 ? ["hour", 3600] : ["day", 86400];
  return <time dateTime={date.toISOString()} title={date.toString()}>
    {absoluteTime.format(date)} <span>· {seconds < 5 ? "just now" : relativeTime.format(-Math.floor(seconds/divisor),unit)}</span>
  </time>;
}

export function TurnLog({ api, path, running }) {
  const [trail, setTrail] = useState([null]);
  const [page, setPage] = useState(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const [now, setNow] = useState(Date.now);
  const viewport = useRef(null);
  const before = trail.at(-1);
  const following = running && before == null;
  const current = page?.before === before ? page : null;

  useEffect(() => {
    const clock = setInterval(() => setNow(Date.now()), 10000);
    return () => clearInterval(clock);
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    let timer, known;
    async function read() {
      const query = new URLSearchParams();
      if (before != null) query.set("before", before);
      if (known != null) query.set("known", known);
      try {
        const data = await api(path + "?" + query, undefined, controller.signal);
        if (controller.signal.aborted) return;
        if (!data.unchanged) {
          known = data.entries.some(row => row.line != null && row.at == null) && running ? undefined : data.end;
          setPage({ ...data, before });
        }
        setNow(Date.now());
        setError("");
      } catch (e) {
        if (!controller.signal.aborted) setError(e.message);
      }
      if (following && !controller.signal.aborted) timer = setTimeout(read, 2000);
    }
    setError("");
    read();
    return () => { controller.abort(); clearTimeout(timer); };
  }, [api, path, before, following, retry, running]);

  function navigate(next) {
    setTrail(next);
    viewport.current?.scrollTo(0, 0);
  }
  function latest() {
    navigate([null]);
  }
  function freeze() {
    if (current && before == null) {
      setPage({ ...current, before: current.end });
      setTrail([current.end]);
    }
  }
  function older() {
    const anchors = before == null ? [current.end] : trail;
    navigate([...anchors, current.older]);
  }
  return <section aria-label="Turn log" className="paged-log">
    <nav className="log-controls" aria-label="Log pages">
      <div>
        <Button size="sm" variant="outline" disabled={before == null} onClick={latest}><ChevronsLeft/> Latest</Button>
        <Button size="sm" variant="outline" disabled={trail.length < 2} onClick={() => navigate(trail.slice(0,-1))}><ChevronLeft/>Newer</Button>
        <Button size="sm" variant="outline" disabled={current?.older == null} onClick={older}>Older<ChevronRight/></Button>
      </div>
      {running && <Button size="sm" variant="ghost" disabled={!current} onClick={following ? freeze : latest}>
        {following ? <Pause/> : <Play/>}{following ? "Pause updates" : "Go live"}
      </Button>}
    </nav>
    <div className="log-page-label" role="status">Page {trail.length} · Newest first{following ? " · Live" : ""}</div>
    {error && <p role="alert">{error} <Button variant="outline" onClick={() => setRetry(n => n + 1)}>Try again</Button></p>}
    {!current && !error && <Loading>Loading log…</Loading>}
    <div className="log-messages" ref={viewport} tabIndex={0} role="region" aria-label="Log messages"
      onScroll={e => { if (following && e.currentTarget.scrollTop > 80) freeze(); }}>
      {current && !current.entries.length && <p>This turn has no log output yet.</p>}
      {current?.entries.map(record => {
        const rows = messageRows(record);
        if (!rows.length) return null;
        const important = rows.some(row => /^(Agent|Text|Message|Result|Error|Standard error|Output)$/i.test(row.title) || (row.title === "Exit code" && row.body !== "0"));
        return <article key={record.id} className="log-event">
          <Disclosure defaultOpen={important} title={<><strong title={rows[0].title}>{rows[0].title}</strong><LogTime at={record.at} now={now}/></>}>
          {rows.map(({title,body,kind},i) => <React.Fragment key={i}>
            {i>0 && <h5>{title}</h5>}<LogBody body={body} kind={kind}/>
          </React.Fragment>)}
        </Disclosure></article>;
      })}
    </div>
  </section>;
}
