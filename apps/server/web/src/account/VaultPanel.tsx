import { useRef } from "react";

import { Pending, buttonClass, inputClass, secondaryClass } from "../components/forms";
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
        "Delete the Apple session and signing assets saved in this browser? This cannot be undone.",
      )
    ) {
      await accountController.forget();
    }
  });

  return (
    <section aria-labelledby="vault-title" className="border-t border-text/10 pt-6">
      <h3 id="vault-title" className="text-base font-semibold">
        {account.vaultExists ? "Unlock saved session" : "Set a browser passphrase"}
      </h3>
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
            Browser passphrase
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
          {!account.vaultExists && (
            <p className="text-xs text-muted">
              Use at least 12 characters. This passphrase encrypts your Apple session and signing
              assets in this browser. AppRelay can’t recover it.
            </p>
          )}
          <button type="submit" className={buttonClass + " w-full"}>
            <Pending
              active={persist.isPending}
              label={account.vaultExists ? "Unlocking…" : "Saving…"}
            >
              {account.vaultExists ? "Unlock" : "Continue"}
            </Pending>
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
