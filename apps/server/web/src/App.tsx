import { AccountWorkspace } from "./account/AccountWorkspace";

export function App() {
  return (
    <div className="mx-auto min-h-svh max-w-xl px-6 py-10 sm:py-16">
      <header className="mb-10">
        <a href="/" className="text-lg font-semibold tracking-tight">
          AppRelay
        </a>
        <h1 className="mt-6 text-3xl font-semibold tracking-tight">Sign your app.</h1>
        <p className="mt-3 text-sm leading-6 text-stone-600">
          Sign in, choose an IPA, then pair your iPhone.
        </p>
      </header>
      <main id="main" className="rounded-2xl border border-stone-200 bg-white p-6 sm:p-8">
        <AccountWorkspace />
      </main>
      <p className="mt-5 text-xs leading-5 text-stone-500">
        Experimental signing. Successful signing does not guarantee installation or launch.
      </p>
    </div>
  );
}
