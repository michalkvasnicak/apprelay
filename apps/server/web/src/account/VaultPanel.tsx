import { useRef } from "react";

import { buttonClass, inputClass, secondaryClass } from "../components/forms";
import { accountController } from "./controller";
import { inputValue, useAccountAction, useAccountBusy, type AccountSnapshot } from "./hooks";

export function VaultPanel({ account }: { account: AccountSnapshot }) {
  const form = useRef<HTMLFormElement>(null);
  const busy = useAccountBusy();
  const persist = useAccountAction(() => {
    const passphrase = inputValue(form, "passphrase", true);
    return account.vaultExists
      ? accountController.unlock(passphrase)
      : accountController.save(passphrase);
  });
  const forget = useAccountAction(async () => {
    if (
      window.confirm(
        "Delete the Apple session and signing assets saved on this device? This cannot be undone and does not revoke assets at Apple.",
      )
    ) {
      await accountController.forget();
    }
  });

  return (
    <section aria-labelledby="vault-title">
      <h3 id="vault-title" className="text-sm font-semibold">
        {account.vaultExists ? "Unlock your saved account" : "Keep your account on this device"}
      </h3>
      <p className="mt-2 text-xs leading-5 text-muted">
        Your session and signing key are encrypted in this browser. Choose a separate passphrase,
        not your Apple password. It cannot be recovered.
      </p>
      <form
        ref={form}
        className="mt-4"
        onSubmit={(event) => {
          event.preventDefault();
          persist.mutate();
        }}
      >
        <fieldset disabled={busy} className="space-y-4">
          <label className="block text-sm font-medium">
            Device passphrase
            <input
              name="passphrase"
              type="password"
              autoComplete={account.vaultExists ? "current-password" : "new-password"}
              minLength={12}
              maxLength={1024}
              required
              className={inputClass}
            />
          </label>
          <button type="submit" className={buttonClass}>
            {persist.isPending ? "Opening…" : "Continue"}
          </button>
        </fieldset>
      </form>
      <button
        type="button"
        disabled={busy}
        onClick={() => forget.mutate()}
        className={secondaryClass + " mt-4"}
      >
        Use another Apple account
      </button>
    </section>
  );
}
