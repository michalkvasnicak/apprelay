import "./sentry";

import * as Sentry from "@sentry/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import "./styles.css";

const queryClient = new QueryClient();

const root = document.getElementById("root");
if (!root) {
  throw new Error("Missing application root");
}

createRoot(root, {
  onUncaughtError: Sentry.reactErrorHandler(),
  onRecoverableError: Sentry.reactErrorHandler(),
}).render(
  <StrictMode>
    <Sentry.ErrorBoundary
      fallback={
        <main role="alert" className="mx-auto max-w-xl px-6 py-10">
          <h1 className="text-xl font-semibold">Something went wrong.</h1>
          <p className="mt-3">Reload the page to try again.</p>
          <button
            type="button"
            className="mt-5 rounded-lg bg-primary px-4 py-2 text-background"
            onClick={() => window.location.reload()}
          >
            Reload
          </button>
        </main>
      }
    >
      <QueryClientProvider client={queryClient}>
        <App />
      </QueryClientProvider>
    </Sentry.ErrorBoundary>
  </StrictMode>,
);
