import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import { AccountError, accountController } from "../account/controller";
import { useAccountAction, useAccountBusy } from "../account/hooks";
import { Pending, Steps, buttonClass, inputClass } from "../components/forms";
import { selectTargetDevice, type PairedDevice } from "../device";
import { PairingError, type PairingController } from "../pairing/controller";
import { SignedResult, type InstallProgress } from "./SignedResult";
import { signingController } from "./controller";
import { useSigning } from "./hooks";
import { queueInstall, uploadSignedIpa } from "./install";
import { SigningError } from "./types";

export function SigningPanel({
  file: sharedFile,
  device,
  pairing,
  onFileSelected,
}: {
  file?: File;
  device: PairedDevice;
  pairing: PairingController;
  onFileSelected: (file: File) => void;
}) {
  const client = useQueryClient();
  const busy = useAccountBusy();
  const state = useSigning();
  const [progress, setProgress] = useState("");
  const [installProgress, setInstallProgress] = useState<InstallProgress>({ stage: "preparing" });
  const uploadedJob = useRef<string | null>(null);
  const submitInstall = async () => {
    try {
      if (!uploadedJob.current) {
        setInstallProgress({ stage: "preparing" });
        uploadedJob.current = await uploadSignedIpa(
          signingController.signedFile(),
          pairing.installAuthorization(),
          (percent) => setInstallProgress({ stage: "uploading", percent }),
        );
      }
      // Completion is idempotent: retry the same job if the response is lost.
      setInstallProgress({ stage: "queueing" });
      await queueInstall(uploadedJob.current, pairing.installAuthorization());
      setInstallProgress({ stage: "queued" });
    } catch (error) {
      setInstallProgress({ stage: "failed" });
      throw new AccountError(
        error instanceof SigningError || error instanceof PairingError
          ? error.message
          : "Could not submit the install. Check your connection and try again.",
      );
    }
  };
  const retryInstall = useAccountAction(submitInstall);
  const changed = () => {
    void client.invalidateQueries({ queryKey: ["signing"] });
  };
  const prepare = useAccountAction(async () => {
    const file = sharedFile;
    if (!file) {
      throw new AccountError("Choose an IPA first");
    }
    try {
      try {
        selectTargetDevice(device);
      } catch {
        throw new AccountError("Pair your iPhone again before signing");
      }
      setProgress("Inspecting IPA…");
      await signingController.inspect(file, changed);
      const bundleId = signingController.snapshot().bundleId;
      if (!bundleId) {
        throw new AccountError("Could not read the app bundle identifier");
      }
      setProgress("Preparing signing certificate…");
      await accountController.certificate();
      setProgress("Provisioning your app…");
      await accountController.provision(bundleId);
      setProgress("Signing your app…");
      await signingController.sign(changed);
    } catch (error) {
      signingController.reset();
      changed();
      throw new AccountError(
        error instanceof AccountError || error instanceof SigningError
          ? error.message
          : "Could not prepare this app. Check your Apple session and target device, then try again.",
      );
    } finally {
      setProgress("");
    }
    await submitInstall();
  });

  if (state.data?.status === "signed") {
    return (
      <SignedResult
        progress={installProgress}
        busy={busy}
        retrying={retryInstall.isPending}
        error={retryInstall.error}
        onRetry={() => retryInstall.mutate()}
      />
    );
  }

  const message = state.data?.status === "signing" ? state.data.message || progress : progress;

  return (
    <section aria-labelledby="sign-title" className="space-y-5">
      <Steps current={2} />
      <h2 id="sign-title" className="text-xl font-semibold tracking-tight">
        {sharedFile ? "Sign this app" : "Choose an IPA"}
      </h2>
      <div className="rounded-xl border border-text/10 bg-background/50 px-4 py-3 text-sm">
        <p className="font-medium">{device.name || "Paired iPhone"}</p>
        <p className="break-all text-muted">{device.model}</p>
      </div>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          prepare.mutate();
        }}
      >
        <fieldset disabled={busy} className="space-y-4">
          {!sharedFile && (
            <label className="block text-sm font-medium">
              IPA file
              <input
                type="file"
                accept=".ipa"
                required
                className={
                  inputClass +
                  " file:mr-3 file:rounded-lg file:border-0 file:bg-text/10 file:px-3 file:py-1.5 file:text-text"
                }
                onChange={(event) => {
                  const file = event.currentTarget.files?.[0];
                  if (file) {
                    onFileSelected(file);
                  }
                }}
              />
            </label>
          )}
          <button type="submit" className={buttonClass + " w-full"}>
            <Pending active={prepare.isPending} label={message || "Signing…"}>
              Sign and install
            </Pending>
          </button>
        </fieldset>
      </form>
      {prepare.isPending && (
        <p role="status" className="text-center text-xs text-muted">
          Keep this page open.
        </p>
      )}
      <details className="text-xs leading-5 text-muted">
        <summary className="cursor-pointer">Limits</summary>
        <p className="mt-2">
          Up to 100 MiB compressed, 256 MiB expanded. Basic development apps only: no extensions,
          frameworks, custom capabilities, or encrypted binaries.
        </p>
      </details>
    </section>
  );
}
