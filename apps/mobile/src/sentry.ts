import * as Sentry from "@sentry/react-native";
import type { ComponentType } from "react";

const dsn = process.env.EXPO_PUBLIC_SENTRY_DSN?.trim();

if (dsn) {
  Sentry.init({
    dsn,
    environment: process.env.EXPO_PUBLIC_SENTRY_ENVIRONMENT?.trim() || undefined,
    sendDefaultPii: false,
    defaultIntegrations: false,
    integrations: [
      Sentry.reactNativeErrorHandlersIntegration(),
      Sentry.dedupeIntegration(),
      Sentry.nativeReleaseIntegration(),
      Sentry.createReactNativeRewriteFrames(),
      Sentry.debugMetaIntegration(),
    ],
    enableAutoSessionTracking: false,
    enableAutoPerformanceTracing: false,
    enableAppStartTracking: false,
    enableNativeFramesTracking: false,
    enableStallTracking: false,
    enableWatchdogTerminationTracking: false,
    enableAppHangTracking: false,
    enableMemoryIntrospection: false,
    enableCaptureFailedRequests: false,
    enableAutoBreadcrumbTracking: false,
    enableNetworkBreadcrumbs: false,
    enableNetworkEventBreadcrumbs: false,
    enableAppLifecycleBreadcrumbs: false,
    enableActivityLifecycleBreadcrumbs: false,
    enableSystemEventBreadcrumbs: false,
    enableAppComponentBreadcrumbs: false,
    enableUserInteractionBreadcrumbs: false,
    maxBreadcrumbs: 0,
    attachScreenshot: false,
    attachViewHierarchy: false,
    enableLogs: false,
    tracePropagationTargets: [],
    beforeBreadcrumb: () => null,
    beforeSend(event) {
      // Preserve error messages, but omit user and request data, including IP fields.
      delete event.request;
      delete event.user;
      delete event.breadcrumbs;
      delete event.extra;
      for (const exception of event.exception?.values ?? []) {
        delete exception.mechanism?.data;
        for (const frame of exception.stacktrace?.frames ?? []) {
          delete frame.vars;
          delete frame.pre_context;
          delete frame.post_context;
          delete frame.context_line;
        }
      }
      return event;
    },
    beforeSendLog: () => null,
  });
}

export function withErrorBoundary(App: ComponentType) {
  return dsn ? Sentry.withErrorBoundary(App, {}) : App;
}
