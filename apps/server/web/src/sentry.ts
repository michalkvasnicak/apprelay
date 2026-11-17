import * as Sentry from "@sentry/react";

const dsn = import.meta.env.VITE_SENTRY_DSN;

if (import.meta.env.PROD && dsn) {
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
