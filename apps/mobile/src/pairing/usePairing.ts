import * as Sentry from "@sentry/react-native";
import { scheduleNotificationAsync } from "expo-notifications";
import { useEffect } from "react";
import { AppState, Linking } from "react-native";

import { createApiClient } from "../api/client";
import type { DeviceRegistration } from "../vpn/config";

const pairingScheme = process.env.EXPO_PUBLIC_APP_SCHEME;
const maxRetries = 3;
const requestTimeoutMs = 15_000;

function notifyPairingFailure() {
  if (AppState.currentState !== "active") {
    return;
  }
  void scheduleNotificationAsync({
    content: { title: "Pairing couldn't be started." },
    trigger: null,
  }).catch(() => {
    // Notifications may be unavailable; pairing failures are already reported to Sentry.
  });
}

function pairingToken(value: string): string | null {
  if (!pairingScheme) {
    return null;
  }
  try {
    const url = new URL(value);
    const tokens = url.searchParams.getAll("token");
    if (
      url.protocol !== `${pairingScheme}:` ||
      url.hostname !== "pair" ||
      (url.pathname !== "" && url.pathname !== "/") ||
      url.username ||
      url.password ||
      url.port ||
      url.hash ||
      [...url.searchParams.keys()].some((key) => key !== "token") ||
      tokens.length !== 1 ||
      !/^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/.test(tokens[0] ?? "")
    ) {
      return null;
    }
    return tokens[0]!;
  } catch {
    return null;
  }
}

async function startPairing(
  token: string,
  device: DeviceRegistration,
  signal: AbortSignal,
): Promise<boolean> {
  const api = createApiClient();
  // Same-device claims are idempotent, including when the first response was lost.
  for (let attempt = 1; attempt <= 1 + maxRetries; attempt++) {
    if (signal.aborted) {
      return false;
    }
    const request = new AbortController();
    const abortRequest = () => request.abort();
    signal.addEventListener("abort", abortRequest);
    const timeout = setTimeout(abortRequest, requestTimeoutMs);
    let status: number | undefined;
    try {
      const { response } = await api.POST("/v1/devices/{device_id}/pairing", {
        params: { path: { device_id: device.device_id } },
        headers: { Authorization: `Bearer ${device.token}` },
        body: { token },
        signal: request.signal,
      });
      if (response.status === 201) {
        return true;
      }
      status = response.status;
    } catch {
      // Report below without forwarding a fetch error that might include credentials.
    } finally {
      clearTimeout(timeout);
      signal.removeEventListener("abort", abortRequest);
    }
    if (signal.aborted) {
      return false;
    }
    let failure = "request";
    if (status !== undefined) {
      failure = "http";
    } else if (request.signal.aborted) {
      failure = "timeout";
    }
    Sentry.captureException(new Error(`Pairing POST failed on attempt ${attempt}`), {
      tags: {
        operation: "start_pairing",
        attempt: String(attempt),
        failure,
        ...(status !== undefined ? { http_status: String(status) } : {}),
      },
    });
    if (attempt <= maxRetries) {
      await new Promise<void>((resolve) => setTimeout(resolve, 1000 * 2 ** (attempt - 1)));
    }
  }
  return false;
}

export function usePairing(register: () => Promise<DeviceRegistration>) {
  useEffect(() => {
    const controller = new AbortController();
    const { signal } = controller;
    const inFlight = new Set<string>();

    async function receive(url: string) {
      const token = pairingToken(url);
      if (signal.aborted || !token || inFlight.has(token)) {
        return;
      }
      inFlight.add(token);
      try {
        const device = await register();
        if (!(await startPairing(token, device, signal)) && !signal.aborted) {
          notifyPairingFailure();
        }
      } catch {
        if (!signal.aborted) {
          Sentry.captureException(new Error("Could not prepare pairing request"), {
            tags: { operation: "start_pairing" },
          });
          notifyPairingFailure();
        }
      } finally {
        inFlight.delete(token);
      }
    }

    const subscription = Linking.addEventListener("url", ({ url }) => void receive(url));
    void Linking.getInitialURL().then(
      (url) => {
        if (url) {
          void receive(url);
        }
      },
      () => {
        // Without a link, there is no pairing attempt to start.
      },
    );
    return () => {
      controller.abort();
      subscription.remove();
    };
  }, [register]);
}
