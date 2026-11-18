import { buttonClass, secondaryClass } from "../components/forms";

export type InstallProgress =
  | { stage: "preparing" | "queueing" | "queued" | "failed" }
  | { stage: "uploading"; percent: number };

export function SignedResult({
  progress,
  busy,
  onRetry,
}: {
  progress: InstallProgress;
  busy: boolean;
  onRetry: () => void;
}) {
  const queued = progress.stage === "queued";
  const failed = progress.stage === "failed";
  return (
    <section aria-labelledby="signed-title">
      <h2 id="signed-title" className="text-lg font-semibold">
        Your app is signed
      </h2>
      <p className="mt-3 text-sm leading-6 text-muted">
        Keep your iPhone connected to the AppRelay VPN for installation.
      </p>
      {progress.stage === "uploading" ? (
        <div className="mt-4 text-sm text-muted">
          <label className="block" htmlFor="install-upload">
            Uploading signed IPA… {progress.percent}%
          </label>
          <progress
            id="install-upload"
            className="mt-2 w-full"
            value={progress.percent}
            max={100}
          />
        </div>
      ) : (
        !failed && (
          <p role="status" className="mt-4 text-sm text-muted">
            {progress.stage === "preparing" && "Preparing upload…"}
            {progress.stage === "queueing" && "Upload complete. Queueing install…"}
            {queued &&
              "Install queued. Your iPhone must stay connected while the app is delivered."}
          </p>
        )
      )}
      {(failed || queued) && (
        <div className="mt-5 flex flex-wrap items-center gap-5">
          {failed && (
            <button type="button" disabled={busy} className={buttonClass} onClick={onRetry}>
              Retry install
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
