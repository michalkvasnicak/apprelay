import { AccountWorkspace } from "./account/AccountWorkspace";
import { SharedArtifact } from "./artifacts/SharedArtifact";

export function App() {
  return (
    <div className="mx-auto min-h-svh max-w-xl px-6 py-10 sm:py-16">
      <header className="mb-10">
        <a
          href="/"
          className="inline-flex items-center gap-3 text-lg font-semibold tracking-tight text-primary"
        >
          <img src="/logo.svg" alt="" width={48} height={48} />
          AppRelay
        </a>
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">Sign your app.</h1>
        <p className="mt-3 text-sm leading-6 text-muted">
          Sign in to Apple and sign an IPA in your browser.
        </p>
      </header>
      <main id="main" className="rounded-2xl border border-text/15 bg-text/[0.03] p-6 sm:p-8">
        {window.location.pathname.startsWith("/s/") ? (
          <SharedArtifact
            key={window.location.pathname}
            token={window.location.pathname.slice(3)}
          />
        ) : (
          <AccountWorkspace />
        )}
      </main>
      <p className="mt-5 text-xs leading-5 text-muted">
        Experimental signing. Successful signing does not guarantee installation or launch.
      </p>
    </div>
  );
}
