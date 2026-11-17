import { useQueryClient } from "@tanstack/react-query";

import { ActionError, secondaryClass } from "../components/forms";
import { SigningPanel } from "../signing/SigningPanel";
import { signingController } from "../signing/controller";
import { AuthenticationPanel } from "./AuthenticationPanel";
import { VaultPanel } from "./VaultPanel";
import { accountController } from "./controller";
import { useAccount, useAccountAction, useAccountBusy } from "./hooks";

export function AccountWorkspace({ file }: { file?: File }) {
  const account = useAccount();
  const busy = useAccountBusy();
  const client = useQueryClient();
  const lock = useAccountAction(async () => {
    await accountController.lock();
    signingController.reset();
    await client.invalidateQueries({ queryKey: ["signing"] });
  });
  if (account.isPending) {
    return (
      <p role="status" className="text-sm text-muted">
        Opening workspace…
      </p>
    );
  }
  if (account.isError) {
    return (
      <p role="alert" className="text-sm text-error">
        Browser storage is unavailable. Enable localStorage for this site, then reload.
      </p>
    );
  }
  const ready =
    account.data.status === "authenticated" && !!account.data.teamId && account.data.vaultUnlocked;

  return (
    <div className="space-y-6" aria-busy={busy}>
      {file && <p className="break-all text-sm text-muted">App: {file.name}</p>}
      {!ready ? (
        <div className="space-y-6">
          <h2 className="text-lg font-semibold">Sign in to Apple</h2>
          <AuthenticationPanel account={account.data} />
          {(account.data.vaultExists ||
            (account.data.status === "authenticated" && account.data.teamId)) && (
            <VaultPanel account={account.data} />
          )}
        </div>
      ) : (
        <div className="space-y-6">
          <SigningPanel file={file} />
          <button
            type="button"
            className={secondaryClass}
            disabled={busy}
            onClick={() => lock.mutate()}
          >
            Lock Apple account
          </button>
          <ActionError error={lock.error} />
        </div>
      )}
    </div>
  );
}
