import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import { AccountError, accountController } from "../account/controller";
import { useAccountAction, useAccountBusy } from "../account/hooks";
import { ActionError, inputClass } from "../components/forms";
import { requireProvisionableDevice, targetDevice } from "../device";
import { signingController } from "./controller";
import { useSigning } from "./hooks";
import { SigningError } from "./types";

export function SigningPanel() {
  const input = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const busy = useAccountBusy();
  const state = useSigning();
  const [progress, setProgress] = useState("");
  const changed = () => {
    void client.invalidateQueries({ queryKey: ["signing"] });
  };
  const prepare = useAccountAction(async () => {
    const file = input.current?.files?.[0];
    if (input.current) {
      input.current.value = "";
    }
    if (!file) {
      throw new AccountError("Choose an IPA first");
    }
    try {
      requireProvisionableDevice();
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
          : "Could not prepare this app. Check the target device configuration and try again.",
      );
    } finally {
      setProgress("");
    }
  });

  return (
    <section aria-labelledby="sign-title">
      <h2 id="sign-title" className="text-lg font-semibold">
        Choose an IPA
      </h2>
      <p className="mt-3 text-sm leading-6 text-stone-600">
        We’ll prepare the signing assets and sign your app automatically. The IPA and private
        signing key stay on this device.
      </p>
      <label className="mt-5 block text-sm font-medium">
        IPA file
        <input
          ref={input}
          type="file"
          accept=".ipa"
          disabled={busy || targetDevice.isPlaceholder}
          className={inputClass}
          onChange={() => {
            if (input.current?.files?.length) {
              prepare.mutate();
            }
          }}
        />
      </label>
      {targetDevice.isPlaceholder && (
        <p role="alert" className="mt-3 text-sm text-amber-800">
          Signing requires a target iPhone UDID. Configure VITE_DEVICE_UDID before continuing.
        </p>
      )}
      {prepare.isPending && (
        <p role="status" className="mt-4 text-sm text-stone-600">
          {state.data?.status === "signing" ? state.data.message || progress : progress} Keep this
          page open.
        </p>
      )}
      <ActionError error={prepare.error} />
      <details className="mt-5 text-xs leading-5 text-stone-500">
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
