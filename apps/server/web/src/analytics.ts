import { pairingHandoffPath } from "./pairing/links";

type FlowEvent =
  | "apple_sign_in_started"
  | "apple_sign_in_two_factor_required"
  | "apple_sign_in_completed"
  | "pairing_started"
  | "pairing_attempt_created"
  | "pairing_code_ready"
  | "pairing_completed"
  | "pairing_failed"
  | "signing_started"
  | "signing_completed"
  | "install_queued";

declare global {
  interface Window {
    umami?: {
      track: (
        mapPayload: (defaults: Record<string, unknown>) => Record<string, unknown>,
      ) => void | Promise<unknown>;
    };
  }
}

let queueEvent: ((event: FlowEvent) => void) | undefined;

export function trackFlowEvent(event: FlowEvent) {
  queueEvent?.(event);
}

function analyticsPath(pathname: string) {
  if (pathname === "/" || pathname === "/sign" || pathname === pairingHandoffPath) {
    return pathname;
  }
  if (pathname.startsWith("/s/")) {
    return "/s/shared";
  }
  return null;
}

export function trackPageview() {
  const scriptUrl = import.meta.env.VITE_UMAMI_SCRIPT_URL?.trim();
  const websiteId = import.meta.env.VITE_UMAMI_WEBSITE_ID?.trim();
  const url = analyticsPath(window.location.pathname);
  if (!import.meta.env.PROD || !scriptUrl || !websiteId || !url) {
    return;
  }

  const script = document.createElement("script");
  script.src = scriptUrl;
  script.dataset.websiteId = websiteId;
  // Pairing and share URLs can contain live tokens. Never send an automatic pageview.
  script.dataset.autoTrack = "false";
  script.referrerPolicy = "no-referrer";
  const pending: FlowEvent[] = [];
  queueEvent = (event) => {
    if (pending.length < 32) {
      pending.push(event);
    }
  };
  script.addEventListener(
    "error",
    () => {
      queueEvent = undefined;
    },
    { once: true },
  );
  script.addEventListener(
    "load",
    () => {
      // Keep the pageview and flow events in order, even if a user starts before Umami loads.
      let tail = Promise.resolve();
      function send(name?: FlowEvent) {
        tail = tail.then(async () => {
          try {
            await window.umami?.track((defaults) => ({
              ...defaults,
              url,
              referrer: "",
              ...(name ? { name } : {}),
            }));
          } catch {
            // Analytics must never interrupt a flow.
          }
        });
      }
      send();
      pending.forEach(send);
      pending.length = 0;
      queueEvent = send;
    },
    { once: true },
  );
  document.head.append(script);
}
