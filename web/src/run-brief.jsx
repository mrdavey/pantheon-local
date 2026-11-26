import React, { useEffect, useState } from "react";
import { Download } from "lucide-react";
import { Button } from "./components/ui/button";
import { Dialog } from "./dialog";
import { Markdown } from "./markdown";
import { Loading } from "./status";

const imageTypes = {
  avif: "image/avif",
  bmp: "image/bmp",
  gif: "image/gif",
  jpeg: "image/jpeg",
  jpg: "image/jpeg",
  png: "image/png",
  webp: "image/webp",
};
const textTypes = /^(txt|md|markdown|csv|tsv|json|log|toml|xml|ya?ml)$/;
const textLimit = 64 * 1024;

function BriefAttachment({ name, path, api }) {
  const [file, setFile] = useState(null);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [brokenImage, setBrokenImage] = useState(false);
  const extension = name.split(".").at(-1).toLowerCase();
  useEffect(() => {
    const request = new AbortController();
    let url;
    setError("");
    api(path, undefined, request.signal)
      .then(({ data }) => {
        if (request.signal.aborted) return;
        const bytes = Uint8Array.from(atob(data), (character) => character.charCodeAt(0));
        const type = Object.hasOwn(imageTypes, extension) ? imageTypes[extension] : null;
        const truncated = bytes.length > textLimit;
        let text = null;
        if (!type && textTypes.test(extension)) {
          try {
            text = new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(0, textLimit), {
              stream: truncated,
            });
          } catch {
            // Offer a download when an attachment is not UTF-8.
          }
        }
        url = URL.createObjectURL(new Blob([bytes], { type: type || "application/octet-stream" }));
        setFile({ url, type, text, truncated });
      })
      .catch((problem) => {
        if (!request.signal.aborted) setError(problem.message);
      });
    return () => {
      request.abort();
      if (url) URL.revokeObjectURL(url);
    };
  }, [api, attempt, extension, path]);
  return (
    <li className="brief-attachment">
      <div className="brief-attachment-heading">
        <strong>{name}</strong>
        {file && (
          <Button asChild size="sm" variant="outline">
            <a href={file.url} download={name} aria-label={`Download ${name}`}>
              <Download />
              Download
            </a>
          </Button>
        )}
      </div>
      {error ? (
        <p role="alert">
          {error}{" "}
          <Button variant="outline" size="sm" onClick={() => setAttempt(attempt + 1)}>
            Retry
          </Button>
        </p>
      ) : !file ? (
        <Loading>Loading attachment…</Loading>
      ) : file.type && !brokenImage ? (
        <img src={file.url} alt={name} onError={() => setBrokenImage(true)} />
      ) : file.text != null ? (
        <>
          {/^(md|markdown)$/.test(extension) ? <Markdown>{file.text}</Markdown> : <pre>{file.text}</pre>}
          {file.truncated && <small>Download to read the full file.</small>}
        </>
      ) : (
        <p>Preview unavailable.</p>
      )}
    </li>
  );
}

export function RunBrief({ run, api, onClose }) {
  const files = run.input.attachments || [];
  return (
    <Dialog title="Run brief" className="run-brief-dialog" onClose={onClose}>
      <Markdown>{run.input.prompt}</Markdown>
      {files.length > 0 && (
        <section className="brief-attachments">
          <h3>Attachments</h3>
          <ul aria-label="Brief attachments">
            {files.map((file, index) => (
              <BriefAttachment
                key={file.path}
                name={file.name}
                api={api}
                path={`/runs/${run.id}/brief-attachments/${index}`}
              />
            ))}
          </ul>
        </section>
      )}
    </Dialog>
  );
}
