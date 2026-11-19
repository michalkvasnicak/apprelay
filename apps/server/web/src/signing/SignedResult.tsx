import type { components } from "../api/schema";
import { Alert, Pending, Spinner, buttonClass, secondaryClass } from "../components/forms";

export type InstallProgress =
  | { stage: "preparing" | "queueing" | "queued" | "failed" }
  | { stage: "uploading"; percent: number };

export function SignedResult({
  progress,
  installStatus,
  reconnecting,
  busy,
  retrying,
  error,
  onRetry,
}: {
  progress: InstallProgress;
  installStatus?: components["schemas"]["InstallJobStatus"] | null;
  reconnecting: boolean;
  busy: boolean;
  retrying: boolean;
  error?: Error | null;
  onRetry: () => void;
}) {
  const submitted = progress.stage === "queued";
  const submissionFailed = progress.stage === "failed";
  let title = "Submitting install…";
  let statusMessage: string | undefined;
  if (submitted) {
    if (installStatus === null) {
      title = "Install status unavailable";
      statusMessage = "This job can no longer be tracked. Check your iPhone for the app.";
    } else if (installStatus?.status === "installed") {
      title = "App installed";
      statusMessage = "You can open the app from your iPhone’s Home Screen.";
    } else if (installStatus?.status === "failed") {
      title = "Couldn’t confirm installation";
      statusMessage = "Check your iPhone’s Home Screen before trying another install.";
    } else {
      title = installStatus?.status === "installing" ? "Install in progress" : "Install queued";
      statusMessage =
        "Keep your iPhone on Wi-Fi with AppRelay VPN connected while the install runs.";
    }
  } else if (submissionFailed) {
    title = "Couldn’t submit install";
  }
  return (
    <section aria-labelledby="signed-title" className="space-y-5">
      <div aria-live="polite" aria-atomic="true" className="space-y-2">
        <h2 id="signed-title" className="text-xl font-semibold tracking-tight">
          {title}
        </h2>
        {statusMessage && (
          <p className="text-sm text-muted">
            {statusMessage}
            {reconnecting && " Reconnecting for status updates…"}
          </p>
        )}
      </div>
      {progress.stage === "uploading" && (
        <div className="space-y-2 text-sm text-muted">
          <label className="flex justify-between" htmlFor="install-upload">
            <span>Uploading signed IPA</span>
            <span className="tabular-nums">{progress.percent}%</span>
          </label>
          <progress
            id="install-upload"
            className="h-2 w-full overflow-hidden rounded-full [&::-moz-progress-bar]:bg-primary [&::-webkit-progress-bar]:bg-text/10 [&::-webkit-progress-value]:bg-primary [&::-webkit-progress-value]:transition-all"
            value={progress.percent}
            max={100}
          />
        </div>
      )}
      {(progress.stage === "preparing" || progress.stage === "queueing") && (
        <p role="status" className="flex items-center gap-2.5 text-sm text-muted">
          <Spinner className="size-4 text-primary" />
          {progress.stage === "preparing" ? "Preparing upload…" : "Queueing install…"}
        </p>
      )}
      {submissionFailed && <Alert error={error ?? "Could not submit the install."} />}
      {(submissionFailed || submitted) && (
        <div className="flex flex-wrap gap-3">
          {submissionFailed && (
            <button type="button" disabled={busy} className={buttonClass} onClick={onRetry}>
              <Pending active={retrying} label="Retrying…">
                Retry install
              </Pending>
            </button>
          )}
          {!busy && (
            <a href="/sign" className={secondaryClass}>
              Sign another app
            </a>
          )}
        </div>
      )}
    </section>
  );
}
