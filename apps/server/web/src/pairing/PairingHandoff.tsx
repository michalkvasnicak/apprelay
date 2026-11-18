import { Alert, buttonClass } from "../components/forms";
import { pairingAppLink, pairingConfigured } from "./links";
import { TestFlightInvite } from "./TestFlightInvite";

export function PairingHandoff() {
  const link = pairingAppLink(window.location.hash.slice(1));

  return (
    <section className="space-y-5" aria-labelledby="pairing-handoff-title">
      <h1 id="pairing-handoff-title" className="text-xl font-semibold tracking-tight">
        Pair your iPhone
      </h1>
      {link ? (
        <>
          <a href={link} rel="noreferrer" className={buttonClass + " w-full"}>
            Open AppRelay
          </a>
          <p className="text-sm text-muted">
            Keep the VPN connected, then return to the browser where you started.
          </p>
          <TestFlightInvite />
        </>
      ) : (
        <Alert
          error={
            pairingConfigured
              ? "This pairing link is invalid. Copy a new one from the browser where you started."
              : "Pairing is not configured on this server."
          }
        />
      )}
    </section>
  );
}
