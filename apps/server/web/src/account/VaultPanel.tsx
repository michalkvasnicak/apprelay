import { useRef } from "react";

import { ActionError, buttonClass, inputClass, secondaryClass } from "../components/forms";
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
  const lock = useAccountAction(() => accountController.lock());
  const forget = useAccountAction(async () => {
    if (
      window.confirm(
        "Delete this browser’s saved Apple session, private signing key, certificates, and profiles? This cannot be undone and does not revoke assets at Apple.",
      )
    ) {
      await accountController.forget();
    }
  });
  const canSave = account.status === "authenticated" || account.vaultExists;
  return (
    <section aria-labelledby="vault-title" className="border-t border-stone-100 pt-6">
      <h3 id="vault-title" className="text-sm font-semibold">
        Browser vault
      </h3>
      <p className="mt-2 text-xs leading-5 text-stone-500">
        Optional for sign-in; required before certificate creation so your signing key survives a
        reload. Sessions and signing assets are encrypted in localStorage with a separate
        passphrase. It is not your Apple password. There is no passphrase recovery; browser data can
        be cleared or evicted. Same-origin scripts can access the vault while unlocked.
      </p>
      {account.vaultUnlocked ? (
        <div className="mt-4 flex items-center gap-5">
          <span className="text-sm text-emerald-800">Saved and unlocked</span>
          <button
            type="button"
            disabled={busy}
            onClick={() => lock.mutate()}
            className={secondaryClass}
          >
            Lock
          </button>
        </div>
      ) : (
        canSave && (
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
                Vault passphrase
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
                {persist.isPending
                  ? "Working…"
                  : account.vaultExists
                    ? "Unlock & validate"
                    : "Remember on this browser"}
              </button>
            </fieldset>
          </form>
        )
      )}
      {(account.status === "authenticated" || account.vaultExists) && (
        <button
          type="button"
          disabled={busy}
          onClick={() => forget.mutate()}
          className={secondaryClass + " mt-4 text-red-800"}
        >
          Forget account and local signing assets
        </button>
      )}
      <ActionError error={persist.error ?? lock.error ?? forget.error} />
    </section>
  );
}
