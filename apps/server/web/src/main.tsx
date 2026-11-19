import "./sentry";

import * as Sentry from "@sentry/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { trackPageview } from "./analytics";
import { App } from "./App";
import { buttonClass } from "./components/forms";
import "./styles.css";

const queryClient = new QueryClient();

const root = document.getElementById("root");
if (!root) {
  throw new Error("Missing application root");
}

trackPageview();

createRoot(root, {
  onUncaughtError: Sentry.reactErrorHandler(),
  onRecoverableError: Sentry.reactErrorHandler(),
}).render(
  <StrictMode>
    <Sentry.ErrorBoundary
      fallback={
        <main role="alert" className="mx-auto max-w-md px-6 py-16">
          <h1 className="text-xl font-semibold">Something went wrong</h1>
          <button
            type="button"
            className={buttonClass + " mt-5"}
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
