import { useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { Linking } from "react-native";

import { createApiClient } from "../api/client";
import type { DeviceRegistration } from "../vpn/config";
import { vpn, type VpnStatus } from "../vpn/native";

const pairingScheme = process.env.EXPO_PUBLIC_APP_SCHEME;

class PairingError extends Error {}

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

export function usePairing(status: VpnStatus, register: () => Promise<DeviceRegistration>) {
  const token = useRef<string | null>(null);
  const automaticallyStarted = useRef(false);
  const request = useRef<AbortController | null>(null);
  const retryAt = useRef(0);
  const [available, setAvailable] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const start = useMutation({
    mutationFn: async () => {
      const capability = token.current;
      if (!capability) {
        throw new PairingError("Open the pairing link from your browser first.");
      }
      if (Date.now() < retryAt.current) {
        throw new PairingError("Please wait before retrying the status request.");
      }
      const controller = new AbortController();
      request.current = controller;
      const timeout = setTimeout(() => controller.abort(), 90_000);
      try {
        if ((await vpn.getStatus()) !== "connected") {
          throw new PairingError("Connect the VPN before pairing.");
        }
        const api = createApiClient();
        const { data: attempt, response: statusResponse } = await api.GET(
          "/v1/pairing-attempts/{token}",
          {
            params: { path: { token: capability } },
            signal: controller.signal,
          },
        );
        if (statusResponse.status === 429) {
          const value = statusResponse.headers.get("Retry-After");
          const seconds = value ? Number(value) : NaN;
          const delay = Number.isFinite(seconds)
            ? seconds * 1000
            : Date.parse(value ?? "") - Date.now();
          retryAt.current = Date.now() + (Number.isFinite(delay) ? Math.max(2000, delay) : 10_000);
        }
        if (!attempt || statusResponse.status !== 200) {
          throw new PairingError(
            `Could not check pairing (HTTP ${statusResponse.status}). Return to the browser for its status.`,
          );
        }
        if (attempt.status === "success") {
          return;
        }
        if (attempt.status === "failed" || attempt.status === "expired") {
          throw new PairingError("This attempt has ended. Start a new attempt in the browser.");
        }
        const device = await register();
        controller.signal.throwIfAborted();
        if ((await vpn.getStatus()) !== "connected") {
          throw new PairingError("The VPN disconnected. Reconnect before retrying pairing.");
        }
        await vpn.preparePairing();
        controller.signal.throwIfAborted();
        // The server waits for actual WireGuard readiness; native startup alone is not proof.
        const { response } = await api.POST("/v1/devices/{device_id}/pairing", {
          params: { path: { device_id: device.device_id } },
          headers: { Authorization: `Bearer ${device.token}` },
          body: { token: capability },
          signal: controller.signal,
        });
        if (response.status !== 201) {
          throw new PairingError(
            `Pairing was not accepted (HTTP ${response.status}). Check the browser before retrying.`,
          );
        }
      } catch (error) {
        // Never expose native/fetch errors that might contain the capability URL.
        throw new PairingError(
          error instanceof PairingError
            ? error.message
            : "Could not start pairing. Check VPN and local-network permissions, then check the browser before retrying.",
        );
      } finally {
        clearTimeout(timeout);
        request.current = null;
      }
    },
    retry: false,
    gcTime: 0,
  });

  useEffect(() => {
    let mounted = true;
    function receive(value: string) {
      if (!mounted) {
        return;
      }
      const incoming = pairingToken(value);
      if (!incoming) {
        setNotice("Invalid pairing link. Open a new link from the browser.");
        return;
      }
      if (incoming === token.current) {
        return;
      }
      if (token.current) {
        setNotice(
          "Another pairing link is already open. Dismiss it before opening a different link.",
        );
        return;
      }
      token.current = incoming;
      setAvailable(true);
      setNotice(null);
    }
    const subscription = Linking.addEventListener("url", ({ url }) => receive(url));
    void Linking.getInitialURL()
      .then((url) => {
        if (url) {
          receive(url);
        }
      })
      .catch(() => {
        if (mounted) {
          setNotice("Could not read the pairing link. Open it again from the browser.");
        }
      });
    return () => {
      mounted = false;
      subscription.remove();
      request.current?.abort();
    };
  }, []);

  const { mutate: startPairing } = start;
  useEffect(() => {
    if (available && status === "connected" && !automaticallyStarted.current) {
      automaticallyStarted.current = true;
      startPairing();
    }
  }, [available, status, startPairing]);

  function dismiss() {
    if (start.isPending) {
      return;
    }
    token.current = null;
    automaticallyStarted.current = false;
    setAvailable(false);
    setNotice(null);
    start.reset();
  }

  return { available, notice, start, dismiss };
}
