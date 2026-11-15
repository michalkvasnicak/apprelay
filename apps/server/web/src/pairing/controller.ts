const pairingScheme = import.meta.env.DEV ? "apprelay-dev" : "apprelay";
const statuses = ["pending", "pairing", "success", "failed", "expired"] as const;
export type PairingStatus = (typeof statuses)[number];

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
  private terminal: PairingStatus | null = null;

  link() {
    return this.token && !this.terminal && Date.now() < this.expiresAt
      ? `${pairingScheme}://pair?token=${encodeURIComponent(this.token)}`
      : null;
  }

  async create() {
    if (Date.now() < this.nextCreateAt) {
      throw new PairingError("Please wait before creating another attempt.");
    }
    let response: Response;
    try {
      response = await fetch("/v1/pairing-attempts", {
        method: "POST",
        cache: "no-store",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.timeout(20_000),
      });
    } catch {
      throw new PairingError("Could not create an attempt. No request was retried automatically.");
    }
    if (response.status === 429) {
      this.nextCreateAt = Date.now() + retryDelay(response);
      throw new PairingError("Too many requests. Please wait before trying again.");
    }
    if (response.status !== 201) {
      throw new PairingError(`Could not create an attempt (HTTP ${response.status}).`);
    }
    const data: unknown = await response.json();
    if (
      !data ||
      typeof data !== "object" ||
      !("token" in data) ||
      !("expires_at" in data) ||
      typeof data.token !== "string" ||
      !/^[A-Za-z0-9_-]{43}$/.test(data.token) ||
      typeof data.expires_at !== "string" ||
      !Number.isFinite(Date.parse(data.expires_at))
    ) {
      throw new PairingError("The server returned an invalid attempt.");
    }
    this.token = data.token;
    this.expiresAt = Date.parse(data.expires_at);
    this.nextRequestAt = 0;
    this.terminal = null;
  }

  interval() {
    const delay = Math.max(2000, this.nextRequestAt - Date.now());
    return Math.max(1000, Math.min(delay, this.expiresAt - Date.now()));
  }

  async status(signal: AbortSignal): Promise<PairingStatus> {
    if (this.terminal) {
      return this.terminal;
    }
    if (!this.token || Date.now() >= this.expiresAt) {
      this.finish("expired");
      return "expired";
    }
    if (Date.now() < this.nextRequestAt) {
      throw new PairingError("Status requests are paused. Waiting before retrying.");
    }
    try {
      const response = await fetch(`/v1/pairing-attempts/${encodeURIComponent(this.token)}`, {
        cache: "no-store",
        referrerPolicy: "no-referrer",
        signal: AbortSignal.any([signal, AbortSignal.timeout(15_000)]),
      });
      if (response.status === 429) {
        this.nextRequestAt = Date.now() + retryDelay(response);
        throw new PairingError("Too many status requests. Waiting before retrying.");
      }
      if (response.status === 404) {
        this.finish("expired");
        return "expired";
      }
      if (!response.ok) {
        throw new PairingError(`Status unavailable (HTTP ${response.status}).`);
      }
      const data: unknown = await response.json();
      if (
        !data ||
        typeof data !== "object" ||
        !("status" in data) ||
        !statuses.some((status) => status === data.status)
      ) {
        throw new PairingError("The server returned an invalid status.");
      }
      const status = data.status as PairingStatus;
      if (status !== "pending" && status !== "pairing") {
        this.finish(status);
      }
      return status;
    } catch (error) {
      if (error instanceof PairingError) {
        throw error;
      }
      throw new PairingError("Could not refresh status. This does not mean pairing failed.");
    }
  }

  private finish(status: PairingStatus) {
    this.terminal = status;
    this.token = null;
  }
}
