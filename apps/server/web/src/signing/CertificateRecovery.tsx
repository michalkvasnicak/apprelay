import { useState } from "react";

import { accountController } from "../account/controller";
import { useAccount, useAccountAction, useAccountBusy } from "../account/hooks";
import { Pending, inputClass, secondaryClass } from "../components/forms";

type Certificate = Awaited<ReturnType<typeof accountController.developmentCertificates>>[number];

export function CertificateRecovery() {
  const account = useAccount().data;
  return account?.teamId && account.vaultUnlocked ? (
    <CertificateReplacement key={account.teamId} teamId={account.teamId} />
  ) : null;
}

function CertificateReplacement({ teamId }: { teamId: string }) {
  const busy = useAccountBusy();
  const [certificates, setCertificates] = useState<Certificate[]>();
  const [selectedId, setSelectedId] = useState("");
  const [confirmed, setConfirmed] = useState(false);
  const [replaced, setReplaced] = useState(false);

  const load = useAccountAction(async () => {
    setCertificates(undefined);
    setSelectedId("");
    setConfirmed(false);
    setReplaced(false);
    setCertificates(await accountController.developmentCertificates(teamId));
  });
  const replace = useAccountAction(async (certificateId: string) => {
    setCertificates(undefined);
    setSelectedId("");
    setConfirmed(false);
    await accountController.replaceCertificate(teamId, certificateId);
    setReplaced(true);
  });

  const selected = certificates?.find((cert) => cert.id === selectedId);

  return (
    <details className="rounded-xl border border-text/10 px-4 py-3 text-sm">
      <summary className="cursor-pointer font-medium">Replace development certificate</summary>
      <fieldset disabled={busy} className="mt-3 space-y-3">
        <p className="text-muted">
          If the team is at its certificate limit, you can revoke an old development certificate to
          free a slot for this browser. This does not change the app’s bundle ID.
        </p>
        <button type="button" className={secondaryClass} onClick={() => load.mutate()}>
          <Pending active={load.isPending} label="Loading certificates…">
            {certificates ? "Reload certificates" : "List development certificates"}
          </Pending>
        </button>
        {certificates?.length === 0 && <p role="status">No development certificates found.</p>}
        {!!certificates?.length && (
          <label className="block font-medium">
            Certificate to revoke
            <select
              className={inputClass}
              value={selectedId}
              onChange={(event) => {
                setSelectedId(event.currentTarget.value);
                setConfirmed(false);
              }}
            >
              <option value="">Choose a certificate</option>
              {certificates.map((cert) => (
                <option key={cert.id} value={cert.id}>
                  {cert.machineName ? `${cert.machineName} — ` : ""}
                  {cert.name} · Serial {cert.serialNumber} · Expires{" "}
                  {new Date(cert.expiresAt).toLocaleDateString()}
                  {cert.matchesThisBrowser ? " (this browser)" : ""}
                </option>
              ))}
            </select>
          </label>
        )}
        {selected && (
          <label className="flex items-start gap-2 text-error">
            <input
              type="checkbox"
              className="mt-1 shrink-0"
              checked={confirmed}
              onChange={(event) => setConfirmed(event.currentTarget.checked)}
            />
            <span>
              I understand that revoking certificate {selected.serialNumber} invalidates every
              provisioning profile that includes it. Other browsers or Macs that use it will need a
              valid certificate and new profiles. I authorize a replacement request if Apple does
              not already list a certificate for this browser’s key.
            </span>
          </label>
        )}
        {(selected || replace.isPending) && (
          <button
            type="button"
            className={secondaryClass + " border-error/40 text-error"}
            disabled={!selected || !confirmed}
            onClick={() => {
              if (selected && confirmed) {
                replace.mutate(selected.id);
              }
            }}
          >
            <Pending active={replace.isPending} label="Revoking and preparing certificate…">
              Revoke and replace
            </Pending>
          </button>
        )}
        {replaced && <p role="status">Certificate ready. You can now sign and install.</p>}
      </fieldset>
    </details>
  );
}
