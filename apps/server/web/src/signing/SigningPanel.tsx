import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import { AccountError, accountController } from "../account/controller";
import { useAccountAction, useAccountBusy } from "../account/hooks";
import { buttonClass, inputClass } from "../components/forms";
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
      <SignedResult progress={installProgress} busy={busy} onRetry={() => retryInstall.mutate()} />
    );
  }

  return (
    <section aria-labelledby="sign-title">
      <h2 id="sign-title" className="text-lg font-semibold">
        {sharedFile ? "Sign this app" : "Choose an IPA"}
      </h2>
      <p className="mt-3 text-sm leading-6 text-muted">
        We’ll sign your app in this browser, then upload the signed IPA for installation on your
        paired iPhone. Your private signing key stays in the browser.
      </p>
      <form
        className="mt-5"
        onSubmit={(event) => {
          event.preventDefault();
          prepare.mutate();
        }}
      >
        <fieldset disabled={busy} className="space-y-5">
          {!sharedFile && (
            <label className="block text-sm font-medium">
              IPA file
              <input
                type="file"
                accept=".ipa"
                required
                className={inputClass}
                onChange={(event) => {
                  const file = event.currentTarget.files?.[0];
                  if (file) {
                    onFileSelected(file);
                  }
                }}
              />
            </label>
          )}
          <div className="text-sm">
            <p className="font-medium">Target: {device.name || "Paired iPhone"}</p>
            <p className="break-all text-muted">{device.model}</p>
            <p className="break-all text-muted">UDID: {device.udid}</p>
            <p className="mt-2 text-xs leading-5 text-muted">
              This device will be registered with your Apple development team if it is not already
              registered. For now, its remote-pairing identifier is used as the UDID; hardware UDID
              equivalence has not been verified.
            </p>
          </div>
          <button type="submit" className={buttonClass}>
            {prepare.isPending ? "Signing…" : "Sign and install"}
          </button>
        </fieldset>
      </form>
      {prepare.isPending && (
        <p role="status" className="mt-4 text-sm text-muted">
          {state.data?.status === "signing" ? state.data.message || progress : progress} Keep this
          page open.
        </p>
      )}
      <details className="mt-5 text-xs leading-5 text-muted">
        <summary className="cursor-pointer">Signing limitations</summary>
        <p className="mt-2">
          Up to 100 MiB compressed and 256 MiB expanded. Basic development apps only; extensions,
          frameworks, custom capabilities, and encrypted binaries are unsupported. Apple certificate
          quotas apply; existing certificates are never revoked automatically. Certificate chain and
          profile signature verification are incomplete.
        </p>
      </details>
    </section>
  );
}
