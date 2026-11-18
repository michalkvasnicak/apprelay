import { Alert, Pending, Spinner, buttonClass, secondaryClass } from "../components/forms";

export type InstallProgress =
  | { stage: "preparing" | "queueing" | "queued" | "failed" }
  | { stage: "uploading"; percent: number };

export function SignedResult({
  progress,
  busy,
  retrying,
  error,
  onRetry,
}: {
  progress: InstallProgress;
  busy: boolean;
  retrying: boolean;
  error?: Error | null;
  onRetry: () => void;
}) {
  const queued = progress.stage === "queued";
  const failed = progress.stage === "failed";
  let title = "Installing…";
  if (queued) {
    title = "Install queued";
  } else if (failed) {
    title = "Install failed";
  }
  return (
    <section aria-labelledby="signed-title" className="space-y-5">
      <h2 id="signed-title" className="text-xl font-semibold tracking-tight">
        {title}
      </h2>
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
      {queued && (
        <p role="status" className="text-sm text-muted">
          Keep your iPhone on Wi-Fi with the VPN connected until the app appears.
        </p>
      )}
      {failed && <Alert error={error ?? "Could not submit the install."} />}
      {(failed || queued) && (
        <div className="flex flex-wrap gap-3">
          {failed && (
            <button type="button" disabled={busy} className={buttonClass} onClick={onRetry}>
              <Pending active={retrying} label="Retrying…">
                Retry install
              </Pending>
            </button>
          )}
          {!busy && (
            <a href="/" className={secondaryClass}>
              Sign another app
            </a>
          )}
        </div>
      )}
    </section>
  );
}
