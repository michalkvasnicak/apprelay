import { SigningPanel } from "../signing/SigningPanel";
import { AuthenticationPanel } from "./AuthenticationPanel";
import { ProvisioningPanel } from "./ProvisioningPanel";
import { VaultPanel } from "./VaultPanel";
import { useAccount, useAccountBusy } from "./hooks";

export function AccountWorkspace() {
  const account = useAccount();
  const busy = useAccountBusy();
  if (account.isPending) {
    return (
      <p role="status" className="text-sm text-stone-500">
        Opening workspace…
      </p>
    );
  }
  if (account.isError) {
    return (
      <p role="alert" className="text-sm text-red-800">
        Browser storage is unavailable. Enable localStorage for this site, then reload.
      </p>
    );
  }
  return (
    <div className="space-y-6" aria-busy={busy}>
      <p className="text-xs leading-5 text-amber-900">
        Experimental Apple integration; not live-verified or production-ready. Use only accounts and
        devices you are authorized to manage.
      </p>
      <AuthenticationPanel account={account.data} />
      <VaultPanel account={account.data} />
      {account.data.status === "authenticated" && <ProvisioningPanel account={account.data} />}
      <SigningPanel account={account.data} />
      {busy && (
        <p role="status" className="text-xs text-stone-500">
          Operation in progress. Keep this page open.
        </p>
      )}
    </div>
  );
}
