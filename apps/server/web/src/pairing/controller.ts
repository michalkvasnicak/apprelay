import { api } from "../api/client";
import type { components } from "../api/schema";

import { isPairingToken, pairingAppLink, pairingHandoffPath } from "./links";

type AttemptStatus = components["schemas"]["AttemptStatus"];
export type PairingStatus = AttemptStatus["status"];
type TerminalStatus = Extract<AttemptStatus, { status: "done" | "failed" }>;

export class PairingError extends Error {}

function retryDelay(response: Response): number {
  const value = response.headers.get("Retry-After");
  if (!value) {
    return 10_000;
  }
  const seconds = Number(value);
  const delay = Number.isFinite(seconds) ? seconds * 1000 : Date.parse(value) - Date.now();
  return Number.isFinite(delay) ? Math.max(2000, delay) : 10_000;
}

// Capability lives only here, not in Query keys, variables, results or browser storage.
export class PairingController {
  private token: string | null = null;
  private expiresAt = 0;
  private nextRequestAt = 0;
  private nextCreateAt = 0;
  private terminal: TerminalStatus | null = null;

  private activeToken() {
    return !this.terminal && Date.now() < this.expiresAt ? this.token : null;
  }

  appLink() {
    const token = this.activeToken();
    return token ? pairingAppLink(token) : null;
  }

  handoffLink() {
    const token = this.activeToken();
    // Keep the capability out of HTTP request URLs.
    return token ? `${window.location.origin}${pairingHandoffPath}#${token}` : null;
  }

  async create() {
    if (Date.now() < this.nextCreateAt) {
      throw new PairingError("Please wait before creating another attempt.");
    }
    try {
      const { data, response } = await api.POST("/v1/pairing-attempts", {
        cache: "no-store",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.timeout(20_000),
      });
      if (response.status === 429) {
        this.nextCreateAt = Date.now() + retryDelay(response);
        throw new PairingError("Too many requests. Please wait before trying again.");
      }
      if (response.status !== 201) {
        throw new PairingError(`Could not create an attempt (HTTP ${response.status}).`);
      }
      if (!data || !isPairingToken(data.token)) {
        throw new PairingError("The server returned an invalid attempt.");
      }
      this.token = data.token;
      this.expiresAt = Date.parse(data.expires_at);
      this.nextRequestAt = 0;
      this.terminal = null;
    } catch (error) {
      if (error instanceof PairingError) {
        throw error;
      }
      throw new PairingError("Could not create an attempt. No request was retried automatically.");
    }
  }

  interval() {
    const delay = Math.max(2000, this.nextRequestAt - Date.now());
    return Math.max(1000, Math.min(delay, this.expiresAt - Date.now()));
  }

  async status(signal: AbortSignal): Promise<AttemptStatus> {
    if (this.terminal) {
      return this.terminal;
    }
    if (!this.token || Date.now() >= this.expiresAt) {
      return this.finish({ status: "failed", reason: "timeout" });
    }
    if (Date.now() < this.nextRequestAt) {
      throw new PairingError("Status requests are paused. Waiting before retrying.");
    }
    try {
      const { data, response } = await api.GET("/v1/pairing-attempts/{token}", {
        params: { path: { token: this.token } },
        cache: "no-store",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.any([signal, AbortSignal.timeout(15_000)]),
      });
      if (response.status === 429) {
        this.nextRequestAt = Date.now() + retryDelay(response);
        throw new PairingError("Too many status requests. Waiting before retrying.");
      }
      if (response.status === 404) {
        return this.finish({ status: "failed", reason: "timeout" });
      }
      if (!response.ok) {
        throw new PairingError(`Status unavailable (HTTP ${response.status}).`);
      }
      if (!data) {
        throw new PairingError("The server returned an invalid status.");
      }
      switch (data.status) {
        case "waiting":
        case "connecting":
        case "pairing":
          return data;
        case "failed":
          return this.finish(data);
        case "done": {
          const expiresAt = Date.parse(data.device.expires_at);
          if (!data.device.udid.trim() || !Number.isFinite(expiresAt)) {
            throw new PairingError("The server returned invalid paired device information.");
          }
          if (expiresAt <= Date.now()) {
            return this.finish({ status: "failed", reason: "timeout" });
          }
          return this.finish(data);
        }
        default:
          throw new PairingError("The server returned an invalid status.");
      }
    } catch (error) {
      if (error instanceof PairingError) {
        throw error;
      }
      throw new PairingError("Could not refresh status. This does not mean pairing failed.");
    }
  }

  private finish(status: TerminalStatus) {
    this.terminal = status;
    this.token = null;
    return status;
  }
}
