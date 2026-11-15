import { AccountWorkspace } from "./account/AccountWorkspace";

const steps = [
  {
    number: "01",
    title: "Apple account",
    description: "Sign in and choose your development team.",
  },
  {
    number: "02",
    title: "Signing assets",
    description: "Prepare a certificate and profile for the supplied device.",
  },
  {
    number: "03",
    title: "Sign locally",
    description: "Inspect and sign a supported IPA. Delivery is not yet decided.",
  },
] as const;

export function App() {
  return (
    <div className="mx-auto flex min-h-svh max-w-6xl flex-col px-6 sm:px-10">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-stone-200 py-7">
        <a
          href="/"
          aria-label="AppRelay home"
          className="flex items-center gap-3 font-semibold tracking-tight"
        >
          <span
            aria-hidden="true"
            className="grid size-9 place-items-center rounded-xl bg-stone-900 text-lg text-white"
          >
            a
          </span>
          AppRelay
        </a>
        <span className="rounded-full border border-stone-300 px-3 py-1 text-xs text-stone-600">
          Browser signing · Preview
        </span>
      </header>

      <main id="main" className="flex-1 py-14 sm:py-20">
        <div className="mb-10 max-w-xl">
          <p className="mb-4 text-xs font-semibold tracking-[0.18em] text-emerald-800 uppercase">
            Your browser. Your workspace.
          </p>
          <h1 className="text-4xl leading-tight font-semibold tracking-tight sm:text-5xl">
            A simpler way to sign.
          </h1>
          <p className="mt-5 text-base leading-7 text-stone-600">
            Connect your Apple account and prepare signing assets for your preconfigured device.
            Private signing keys stay in your browser.
          </p>
        </div>

        <div className="grid gap-6 lg:grid-cols-[1fr_19rem]">
          <section
            aria-labelledby="workspace-title"
            className="overflow-hidden rounded-2xl border border-stone-200 bg-white"
          >
            <div className="border-b border-stone-100 px-6 py-5 sm:px-8">
              <h2 id="workspace-title" className="text-sm font-semibold">
                Signing workspace
              </h2>
            </div>
            <div className="px-6 py-8 sm:px-8">
              <AccountWorkspace />
            </div>
          </section>

          <aside
            aria-labelledby="flow-title"
            className="rounded-2xl border border-stone-200 p-6 sm:p-8"
          >
            <h2 id="flow-title" className="text-sm font-semibold">
              The signing flow
            </h2>
            <ol className="mt-7 space-y-7">
              {steps.map((step) => (
                <li key={step.number} className="flex gap-4">
                  <span className="pt-0.5 font-mono text-xs text-stone-400" aria-hidden="true">
                    {step.number}
                  </span>
                  <div>
                    <h3 className="text-sm font-medium">{step.title}</h3>
                    <p className="mt-1 text-xs leading-5 text-stone-500">{step.description}</p>
                  </div>
                </li>
              ))}
            </ol>
          </aside>
        </div>

        <p className="mt-6 max-w-2xl text-xs leading-6 text-stone-500">
          Signing keys are generated locally. Apple authentication uses an HTTPS relay; the relay
          can read Apple tokens in transit. IPA contents stay local; delivery and installation are
          not configured.
        </p>
      </main>

      <footer className="flex flex-wrap items-center justify-between gap-4 border-t border-stone-200 py-6">
        <p className="text-xs text-stone-500">AppRelay · Browser signing workspace</p>
        <div className="flex items-center gap-5">
          <a
            href="/openapi.json"
            className="text-xs text-stone-500 underline decoration-stone-300 underline-offset-4 hover:text-stone-900"
          >
            OpenAPI
          </a>
        </div>
      </footer>
    </div>
  );
}
