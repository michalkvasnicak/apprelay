import { AccountWorkspace } from "./account/AccountWorkspace";
import { SharedArtifact } from "./artifacts/SharedArtifact";
import { PairingHandoff } from "./pairing/PairingHandoff";
import { pairingHandoffPath } from "./pairing/links";

function Brand() {
  return (
    <a
      href="/"
      className="mb-8 inline-flex items-center gap-2.5 text-lg font-semibold tracking-tight text-primary"
    >
      <img src="/logo.svg" alt="" width={36} height={36} />
      AppRelay
    </a>
  );
}

export function App() {
  const path = window.location.pathname;
  const handoff = path === pairingHandoffPath;

  return (
    <div className="mx-auto min-h-svh max-w-lg px-5 py-8 sm:py-14">
      <Brand />
      <main
        id="main"
        className="rounded-3xl border border-text/10 bg-text/[0.04] p-5 shadow-2xl shadow-black/30 backdrop-blur sm:p-8"
      >
        {handoff ? (
          <PairingHandoff />
        ) : path.startsWith("/s/") ? (
          <SharedArtifact key={path} token={path.slice(3)} />
        ) : (
          <AccountWorkspace />
        )}
      </main>
    </div>
  );
}
