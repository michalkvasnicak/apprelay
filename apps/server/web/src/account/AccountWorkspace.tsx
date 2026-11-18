import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { Alert, Pending, Status, Steps, secondaryClass } from "../components/forms";
import { clearTargetDevice } from "../device";
import { SigningWorkspace } from "../signing/SigningWorkspace";
import { signingController } from "../signing/controller";
import { AuthenticationPanel } from "./AuthenticationPanel";
import { VaultPanel } from "./VaultPanel";
import { accountController } from "./controller";
import {
  useAccount,
  useAccountAction,
  useAccountBusy,
  useAccountError,
  useDismissAccountError,
} from "./hooks";

export function AccountWorkspace({ file }: { file?: File }) {
  const [localFile, setLocalFile] = useState<File>();
  const selectedFile = file ?? localFile;
  const account = useAccount();
  const actionError = useAccountError();
  const dismissError = useDismissAccountError();
  const busy = useAccountBusy();
  const client = useQueryClient();
  const lock = useAccountAction(async () => {
    await accountController.lock();
    clearTargetDevice();
    signingController.reset();
    await client.invalidateQueries({ queryKey: ["signing"] });
  });
  if (account.isPending) {
    return <Status>Opening workspace…</Status>;
  }
  if (account.isError) {
    return (
      <Alert
        error="Browser storage is unavailable. Enable localStorage for this site."
        action={
          <button
            type="button"
            className="cursor-pointer font-medium underline underline-offset-4"
            onClick={() => void account.refetch()}
          >
            Retry
          </button>
        }
      />
    );
  }
  const ready =
    account.data.status === "authenticated" && !!account.data.teamId && account.data.vaultUnlocked;

  return (
    <div className="space-y-5" aria-busy={busy}>
      {selectedFile && (
        <p className="truncate rounded-xl bg-background/50 px-3.5 py-2.5 text-sm text-muted">
          <span className="text-text">{selectedFile.name}</span>
        </p>
      )}
      <Alert error={actionError} onDismiss={dismissError} />
      {!ready ? (
        <div className="space-y-6">
          <Steps current={0} />
          <h2 className="text-xl font-semibold tracking-tight">Sign in to Apple</h2>
          <AuthenticationPanel account={account.data} />
          {(account.data.vaultExists ||
            (account.data.status === "authenticated" && account.data.teamId)) && (
            <VaultPanel account={account.data} />
          )}
        </div>
      ) : (
        <div className="space-y-6">
          <SigningWorkspace file={selectedFile} onFileSelected={setLocalFile} />
          <div className="border-t border-text/10 pt-5">
            <button
              type="button"
              className={secondaryClass}
              disabled={busy}
              onClick={() => lock.mutate()}
            >
              <Pending active={lock.isPending} label="Locking…">
                Lock account
              </Pending>
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
