import { buttonClass } from "../components/forms";
import { pairingAppLink, pairingConfigured } from "./links";

export function PairingHandoff() {
  const link = pairingAppLink(window.location.hash.slice(1));

  return (
    <section className="space-y-4" aria-labelledby="pairing-handoff-title">
      <h1 id="pairing-handoff-title" className="text-xl font-semibold">
        Pair your iPhone
      </h1>
      {link ? (
        <>
          <p className="text-sm leading-6 text-muted">
            On the target iPhone, open AppRelay and confirm pairing. Keep its VPN connected until
            pairing completes, then continue in the browser where you started.
          </p>
          <a href={link} rel="noreferrer" className={buttonClass}>
            Open AppRelay
          </a>
        </>
      ) : (
        <p role="alert" className="text-sm text-error">
          {pairingConfigured
            ? "This pairing link is invalid. Copy a new link from the browser where you started."
            : "Pairing is not configured on this server."}
        </p>
      )}
    </section>
  );
}
