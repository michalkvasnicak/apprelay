import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";

import { AccountError, accountController } from "../account/controller";
import { useAccountAction, useAccountBusy } from "../account/hooks";
import { ActionError, buttonClass, inputClass } from "../components/forms";
import { selectTargetDevice } from "../device";
import { SignedResult } from "./SignedResult";
import { signingController } from "./controller";
import { useSigning } from "./hooks";
import { SigningError } from "./types";

export function SigningPanel({ file: sharedFile }: { file?: File }) {
  const input = useRef<HTMLInputElement>(null);
  const udid = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const busy = useAccountBusy();
  const state = useSigning();
  const [progress, setProgress] = useState("");
  const changed = () => {
    void client.invalidateQueries({ queryKey: ["signing"] });
  };
  const prepare = useAccountAction(async () => {
    const file = sharedFile ?? input.current?.files?.[0];
    if (!file) {
      throw new AccountError("Choose an IPA first");
    }
    try {
      try {
        selectTargetDevice(udid.current?.value ?? "");
      } catch {
        throw new AccountError("Enter a valid iPhone hardware UDID before signing");
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
  });

  if (state.data?.status === "signed") {
    return <SignedResult />;
  }

  return (
    <section aria-labelledby="sign-title">
      <h2 id="sign-title" className="text-lg font-semibold">
        {sharedFile ? "Sign this app" : "Choose an IPA"}
      </h2>
      <p className="mt-3 text-sm leading-6 text-muted">
        We’ll prepare the signing assets and sign your app automatically. Signing happens in this
        browser; your private signing key stays here.
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
              <input ref={input} type="file" accept=".ipa" required className={inputClass} />
            </label>
          )}
          <label className="block text-sm font-medium">
            Target iPhone UDID
            <input
              ref={udid}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              pattern="(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{8}-[a-fA-F0-9]{16})"
              maxLength={40}
              required
              className={inputClass}
            />
          </label>
          <p className="text-xs leading-5 text-muted">
            Use the hardware UDID shown in Finder or Apple Devices for the iPhone you intend to
            install on. It will be registered with your Apple development team. This does not pair
            or connect your phone.
          </p>
          <button type="submit" className={buttonClass}>
            {prepare.isPending ? "Signing…" : "Sign app"}
          </button>
        </fieldset>
      </form>
      {prepare.isPending && (
        <p role="status" className="mt-4 text-sm text-muted">
          {state.data?.status === "signing" ? state.data.message || progress : progress} Keep this
          page open.
        </p>
      )}
      <ActionError error={prepare.error} />
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
