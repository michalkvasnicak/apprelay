import { useEffect, useState } from "react";

import { AccountWorkspace } from "../account/AccountWorkspace";
import { Alert, Status, buttonClass, secondaryClass } from "../components/forms";
import { ArtifactError, downloadArtifact } from "./download";

type DownloadState =
  | { status: "loading" }
  | { status: "ready"; file: File }
  | { status: "error"; message: string };

export function SharedArtifact({ token }: { token: string }) {
  const [state, setState] = useState<DownloadState>({ status: "loading" });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    const signal = AbortSignal.any([controller.signal, AbortSignal.timeout(3 * 60_000)]);
    void downloadArtifact(token, signal).then(
      (file) => {
        if (!controller.signal.aborted) {
          setState({ status: "ready", file });
        }
      },
      (error: unknown) => {
        if (!controller.signal.aborted) {
          setState({
            status: "error",
            message: error instanceof ArtifactError ? error.message : "Could not load this app.",
          });
        }
      },
    );
    return () => controller.abort();
  }, [token, attempt]);

  if (state.status === "ready") {
    return <AccountWorkspace file={state.file} />;
  }
  if (state.status === "loading") {
    return <Status>Downloading and verifying IPA…</Status>;
  }
  return (
    <section className="space-y-5">
      <Alert error={state.message} />
      <div className="flex flex-wrap gap-3">
        <button
          type="button"
          className={buttonClass}
          onClick={() => {
            setState({ status: "loading" });
            setAttempt(attempt + 1);
          }}
        >
          Try again
        </button>
        <a href="/sign" className={secondaryClass}>
          Choose a local IPA
        </a>
      </div>
    </section>
  );
}
