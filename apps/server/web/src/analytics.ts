import { pairingHandoffPath } from "./pairing/links";

declare global {
  interface Window {
    umami?: {
      track: (mapPayload: (defaults: Record<string, unknown>) => Record<string, unknown>) => void;
    };
  }
}

function analyticsPath(pathname: string) {
  if (pathname === "/" || pathname === pairingHandoffPath) {
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
  script.addEventListener(
    "load",
    () => {
      window.umami?.track((defaults) => ({ ...defaults, url, referrer: "" }));
    },
    { once: true },
  );
  document.head.append(script);
}
