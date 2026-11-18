import * as Sentry from "@sentry/react";

const dsn = import.meta.env.VITE_SENTRY_DSN;
const enabled = import.meta.env.PROD && dsn;
if (enabled) {
  Sentry.init({
    dsn,
    environment: import.meta.env.VITE_SENTRY_ENVIRONMENT || "production",
    // The build plugin injects the release when SENTRY_RELEASE is provided.
    // Only install error integrations: no console, DOM, or network breadcrumbs,
    // request context, replay, tracing, or logs around Apple credentials/assets.
    defaultIntegrations: false,
    integrations: [
      Sentry.eventFiltersIntegration(),
      Sentry.browserApiErrorsIntegration(),
      Sentry.globalHandlersIntegration(),
      Sentry.dedupeIntegration(),
    ],
    dataCollection: {
      userInfo: false,
      cookies: false,
      httpHeaders: false,
      httpBodies: [],
      urlQueryParams: false,
      stackFrameVariables: false,
    },
    tracePropagationTargets: [],
    beforeSend(event) {
      // Defense in depth against future integrations adding request/user data.
      delete event.request;
      delete event.user;
      delete event.breadcrumbs;
      return event;
    },
    beforeSendLog: () => null,
  });
}

const appleIntegrationFailures = {
  "anisette-setup": "Apple integration: anisette setup failed",
  "anisette-headers": "Apple integration: anisette header refresh failed during sign-in",
  "anisette-rejected":
    "Apple integration: Apple requires anisette reprovisioning or resynchronization",
  "list-teams": "Apple integration: development team lookup failed",
} as const;

// Only report reviewed failure categories, never the underlying error or protocol data.
export function reportAppleIntegrationFailure(stage: keyof typeof appleIntegrationFailures) {
  if (enabled) {
    Sentry.captureMessage(appleIntegrationFailures[stage], {
      level: "error",
      tags: { "apple.integration_stage": stage },
    });
  }
}
