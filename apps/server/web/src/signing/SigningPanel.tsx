import { useQueryClient } from "@tanstack/react-query";
import { useRef } from "react";

import { AccountError } from "../account/controller";
import { useAccountAction, useAccountBusy, type AccountSnapshot } from "../account/hooks";
import { ActionError, buttonClass, inputClass, secondaryClass } from "../components/forms";
import { targetDevice } from "../device";
import { signingController } from "./controller";
import { useSigning } from "./hooks";
import { SigningError } from "./types";

export function SigningPanel({ account }: { account: AccountSnapshot }) {
  const input = useRef<HTMLInputElement>(null);
  const client = useQueryClient();
  const busy = useAccountBusy();
  const state = useSigning();
  const changed = () => {
    void client.invalidateQueries({ queryKey: ["signing"] });
  };
  const inspect = useAccountAction(async () => {
    const file = input.current?.files?.[0];
    if (input.current) {
      input.current.value = "";
    }
    if (!file) {
      throw new AccountError("Choose an IPA first");
    }
    try {
      await signingController.inspect(file, changed);
    } catch (error) {
      throw new AccountError(
        error instanceof SigningError ? error.message : "Local IPA inspection failed",
      );
    }
  });
  const sign = useAccountAction(async () => {
    try {
      await signingController.sign(changed);
    } catch (error) {
      if (error instanceof SigningError) {
        throw new AccountError(error.message);
      }
      throw error;
    }
  });
  const snapshot = state.data;
  const working = snapshot?.status === "inspecting" || snapshot?.status === "signing";
  return (
    <section aria-labelledby="sign-title" className="border-t border-stone-100 pt-6">
      <h3 id="sign-title" className="text-sm font-semibold">
        Local IPA signing
      </h3>
      <p className="mt-2 text-xs leading-5 text-stone-500">
        One app, basic development entitlements, up to 100 MiB compressed and 256 MiB expanded.
        Extensions, frameworks, custom capabilities, and encrypted binaries are not supported. IPA
        contents never leave this browser.
      </p>
      <label className="mt-4 block text-sm font-medium">
        IPA file
        <input
          ref={input}
          type="file"
          accept=".ipa"
          disabled={busy}
          className={inputClass}
          onChange={() => {
            sign.reset();
            inspect.mutate();
          }}
        />
      </label>
      {snapshot?.bundleId && <p className="mt-3 break-all text-sm">App · {snapshot.bundleId}</p>}
      {snapshot?.bundleId &&
        snapshot.status === "ready" &&
        account.profile?.bundleId !== snapshot.bundleId && (
          <p className="mt-2 text-xs text-amber-800">
            Request the matching profile in Signing assets before signing.
          </p>
        )}
      {snapshot?.status === "ready" && (
        <button
          type="button"
          className={buttonClass + " mt-4"}
          disabled={
            busy ||
            !account.vaultUnlocked ||
            targetDevice.isPlaceholder ||
            account.profile?.bundleId !== snapshot.bundleId
          }
          onClick={() => sign.mutate()}
        >
          Sign locally
        </button>
      )}
      {snapshot?.status !== "idle" && (
        <button
          type="button"
          className={secondaryClass + " mt-4 ml-4"}
          onClick={() => {
            signingController.reset();
            changed();
            inspect.reset();
            sign.reset();
          }}
        >
          {working ? "Cancel" : "Clear IPA"}
        </button>
      )}
      {snapshot?.message && (
        <p role="status" className="mt-3 text-xs leading-5 text-stone-600">
          {snapshot.message}
        </p>
      )}
      {snapshot?.status === "signed" && (
        <p className="mt-2 text-xs leading-5 text-amber-800">
          Output generated ({Math.ceil((snapshot.resultBytes ?? 0) / 1024)} KiB), not
          device-verified. Download and installation are intentionally undecided. Clearing or
          reloading discards the result.
        </p>
      )}
      <p className="mt-3 text-xs leading-5 text-amber-900">
        Experimental signer. Certificate chain and profile CMS signature validation are not
        complete; successful processing does not prove the app will install or launch.
      </p>
      <ActionError error={inspect.error} />
      <ActionError error={sign.error} />
    </section>
  );
}
