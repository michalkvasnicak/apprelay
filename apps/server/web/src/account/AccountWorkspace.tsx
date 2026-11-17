import { useState } from "react";

import { PairingWorkspace } from "../pairing/PairingWorkspace";
import { SigningPanel } from "../signing/SigningPanel";
import { useSigning } from "../signing/hooks";
import { AuthenticationPanel } from "./AuthenticationPanel";
import { VaultPanel } from "./VaultPanel";
import { useAccount, useAccountBusy } from "./hooks";

const steps = ["Apple account", "Sign IPA", "Pair iPhone", "Upload"] as const;

export function AccountWorkspace() {
  const account = useAccount();
  const signing = useSigning();
  const busy = useAccountBusy();
  const [paired, setPaired] = useState(false);
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
  let step = 0;
  if (ready) {
    if (signing.data?.status !== "signed") {
      step = 1;
    } else if (paired) {
      step = 3;
    } else {
      step = 2;
    }
  }
  return (
    <div className="space-y-6" aria-busy={busy}>
      <ol
        aria-label="Installation steps"
        className="flex flex-wrap gap-x-4 gap-y-2 text-xs text-muted"
      >
        {steps.map((title, index) => (
          <li
            key={title}
            aria-current={index === step ? "step" : undefined}
            className={index === step ? "font-semibold text-primary" : ""}
          >
            {index + 1}. {title}
          </li>
        ))}
      </ol>
      {step === 0 && (
        <div className="space-y-6">
          <h2 className="text-lg font-semibold">Sign in to Apple</h2>
          <AuthenticationPanel account={account.data} />
          {(account.data.vaultExists ||
            (account.data.status === "authenticated" && account.data.teamId)) && (
            <VaultPanel account={account.data} />
          )}
        </div>
      )}
      {step === 1 && <SigningPanel />}
      {step === 2 && <PairingWorkspace onPaired={() => setPaired(true)} />}
      {step === 3 && (
        <section aria-labelledby="upload-title">
          <h2 id="upload-title" className="text-lg font-semibold">
            Ready to upload
          </h2>
          <p role="status" className="mt-3 text-sm leading-6 text-muted">
            Your app is signed and your iPhone is paired. Upload is not implemented yet; no app has
            been sent to your phone.
          </p>
          <p className="mt-3 text-xs leading-5 text-muted">
            Upload progress will appear here when available. The signed IPA stays in memory until
            you leave or reload this page.
          </p>
        </section>
      )}
    </div>
  );
}
