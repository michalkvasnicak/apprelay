import { useRef } from "react";

import { ActionError, buttonClass, inputClass } from "../components/forms";
import { targetDevice } from "../device";
import { accountController } from "./controller";
import { inputValue, useAccountAction, useAccountBusy, type AccountSnapshot } from "./hooks";

export function ProvisioningPanel({ account }: { account: AccountSnapshot }) {
  const form = useRef<HTMLFormElement>(null);
  const busy = useAccountBusy();
  const certificate = useAccountAction(() => accountController.certificate());
  const provision = useAccountAction(() =>
    accountController.provision(inputValue(form, "bundleId").trim()),
  );
  return (
    <section aria-labelledby="assets-title" className="border-t border-stone-100 pt-6">
      <h3 id="assets-title" className="text-sm font-semibold">
        Signing assets
      </h3>
      <p className="mt-2 text-xs leading-5 text-stone-500">
        Create a browser-local RSA key and request a matching development certificate. This uses
        Apple certificate quotas. No existing certificate is revoked.
      </p>
      {account.certificateExpiresAt ? (
        <p className="mt-4 text-sm text-emerald-800">
          Development certificate saved · expires{" "}
          {new Date(account.certificateExpiresAt).toLocaleDateString()}
        </p>
      ) : (
        <button
          type="button"
          disabled={busy || !account.vaultUnlocked || !account.teamId}
          onClick={() => certificate.mutate()}
          className={buttonClass + " mt-4"}
        >
          {certificate.isPending
            ? "Preparing certificate…"
            : account.certificateRequested
              ? "Recheck certificate request"
              : "Create development certificate"}
        </button>
      )}
      <ActionError error={certificate.error} />
      <div className="mt-6 rounded-lg bg-stone-50 p-4">
        <p className="text-xs font-medium text-stone-700">
          Preconfigured target · {targetDevice.name}
        </p>
        <p className="mt-1 break-all font-mono text-xs text-stone-500">{targetDevice.udid}</p>
        {targetDevice.isPlaceholder && (
          <p className="mt-2 text-xs text-amber-800">
            Placeholder only. Device registration and profile creation are blocked until a real
            hardware UDID is supplied.
          </p>
        )}
      </div>
      <form
        ref={form}
        className="mt-5"
        onSubmit={(event) => {
          event.preventDefault();
          provision.mutate();
        }}
      >
        <fieldset
          disabled={busy || targetDevice.isPlaceholder || !account.certificateExpiresAt}
          className="space-y-4"
        >
          <label className="block text-sm font-medium">
            App bundle identifier
            <input
              name="bundleId"
              placeholder="com.example.app"
              required
              maxLength={255}
              className={inputClass}
            />
          </label>
          <p className="text-xs leading-5 text-stone-500">
            Basic development provisioning only. IPA inspection, capability configuration,
            extensions, and signing are not integrated. No capabilities are silently removed.
          </p>
          <button type="submit" className={buttonClass}>
            {provision.isPending ? "Provisioning…" : "Register device & request profile"}
          </button>
        </fieldset>
      </form>
      {account.profile && (
        <p className="mt-4 text-sm text-emerald-800">
          Profile saved for {account.profile.bundleId} · expires{" "}
          {new Date(account.profile.expiresAt).toLocaleString()}. CMS signature and chain
          verification are still pending; this does not mean the IPA is ready to sign.
        </p>
      )}
      <ActionError error={provision.error} />
    </section>
  );
}
