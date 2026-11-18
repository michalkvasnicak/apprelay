import { hex, sha256 } from "./crypto";
import { base64, text, unbase64 } from "./encoding";
import { decodePlist, encodePlist } from "./plist";
import { relay, requireSuccess } from "./transport";
import type { AnisetteData, AnisetteIdentity } from "./types";
import { integer, record, string } from "./validation";

const clientInfo = "<Mac16,5> <Mac OS X;26.7.2;25G332> <com.apple.AuthKit/1 (com.apple.akd/1.0)>";

async function loadProvider() {
  const response = await fetch("/v1/apple/anisette-provider", {
    credentials: "omit",
    cache: "no-store",
    redirect: "error",
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new Error("Anisette provider configuration is unavailable");
  }
  return new URL(string(record(await response.json()).url));
}

export class Anisette {
  private constructor(
    private readonly identity: AnisetteIdentity,
    private readonly provider: URL,
  ) {}

  static async restore(identity: AnisetteIdentity) {
    if (unbase64(identity.identifier).length !== 16 || !unbase64(identity.adiPb).length) {
      throw new Error("Invalid saved anisette identity");
    }
    return new Anisette(identity, await loadProvider());
  }

  static async create() {
    const provider = await loadProvider();
    const response = await relay(new URL("/v3/client_info", provider).href, "GET", {
      accept: "application/json",
    });
    requireSuccess(response.status);
    const info = record(JSON.parse(text(response.body)));
    const session = new Anisette(
      {
        identifier: base64(crypto.getRandomValues(new Uint8Array(16))),
        adiPb: "",
        userAgent: string(info.user_agent),
      },
      provider,
    );
    await session.provision();
    return session;
  }

  snapshot(): AnisetteIdentity {
    return { ...this.identity };
  }

  private async identityFields() {
    const identifier = unbase64(this.identity.identifier);
    const digits = hex(identifier);
    return {
      localUserID: hex(await sha256(identifier)),
      deviceUniqueIdentifier: `${digits.slice(0, 8)}-${digits.slice(8, 12)}-${digits.slice(12, 16)}-${digits.slice(16, 20)}-${digits.slice(20)}`,
    };
  }

  private async apple(url: string, cpim?: string, post = false) {
    const identity = await this.identityFields();
    const response = await relay(
      url,
      post ? "POST" : "GET",
      {
        "User-Agent": this.identity.userAgent,
        "X-MMe-Client-Info": clientInfo,
        "Content-Type": "text/x-xml-plist",
        "X-Apple-I-MD-LU": identity.localUserID,
        "X-Mme-Device-Id": identity.deviceUniqueIdentifier,
        "X-Apple-I-Client-Time": new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
        "X-Apple-I-TimeZone": "UTC",
        "X-Apple-Locale": "en_US",
        ...(this.identity.routingInfo === undefined
          ? {}
          : { "X-Apple-I-MD-RINFO": String(this.identity.routingInfo) }),
      },
      post ? encodePlist({ Header: {}, Request: cpim ? { cpim } : {} }) : undefined,
    );
    requireSuccess(response.status);
    const parsed = decodePlist(response.body);
    const routing =
      response.headers["x-apple-i-md-rinfo"] ??
      (parsed.Response ? record(parsed.Response)["routing-info"] : undefined);
    if (routing !== undefined) {
      this.identity.routingInfo = integer(routing);
    }
    return parsed;
  }

  private async provision() {
    const lookup = await this.apple("https://gsa.apple.com/grandslam/GsService2/lookup");
    const urls = record(lookup.urls);
    const start = string(urls.midStartProvisioning);
    const finish = string(urls.midFinishProvisioning);
    // The server independently restricts both Apple destinations. The provider
    // receives only its anisette protocol material, never Apple account tokens.
    await new Promise<void>((resolve, reject) => {
      const url = new URL("/v3/provisioning_session", this.provider);
      url.protocol = "wss:";
      const socket = new WebSocket(url);
      let phase: "identifier" | "start" | "finish" | "success" = "identifier";
      let settled = false;
      const complete = (success: boolean) => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timer);
        socket.onmessage = null;
        socket.onclose = null;
        socket.onerror = null;
        socket.close();
        if (success) {
          resolve();
        } else {
          reject(new Error("Anisette provisioning failed; start a new sign-in"));
        }
      };
      const timer = setTimeout(() => complete(false), 240_000);
      let queue = Promise.resolve();
      socket.onerror = () => complete(false);
      socket.onclose = () => complete(false);
      socket.onmessage = (event: MessageEvent<unknown>) => {
        queue = queue
          .then(async () => {
            if (settled) {
              return;
            }
            if (typeof event.data !== "string" || event.data.length > 2_800_000) {
              throw new Error("Invalid anisette message");
            }
            const message = record(JSON.parse(event.data));
            let outgoing: object;
            if (message.result === "GiveIdentifier" && phase === "identifier") {
              outgoing = { identifier: this.identity.identifier };
              phase = "start";
            } else if (message.result === "GiveStartProvisioningData" && phase === "start") {
              const reply = record((await this.apple(start, undefined, true)).Response);
              outgoing = { spim: string(reply.spim) };
              phase = "finish";
            } else if (message.result === "GiveEndProvisioningData" && phase === "finish") {
              const cpim = base64(unbase64(string(message.cpim)));
              const reply = record((await this.apple(finish, cpim, true)).Response);
              outgoing = { ptm: string(reply.ptm), tk: string(reply.tk) };
              phase = "success";
            } else if (message.result === "ProvisioningSuccess" && phase === "success") {
              this.identity.adiPb = base64(unbase64(string(message.adi_pb)));
              if (!this.identity.adiPb) {
                throw new Error("Missing anisette identity");
              }
              complete(true);
              return;
            } else {
              throw new Error("Unexpected anisette message");
            }
            if (!settled) {
              socket.send(JSON.stringify(outgoing));
            }
          })
          .catch(() => complete(false));
      };
    });
  }

  async fresh(): Promise<AnisetteData> {
    const response = await relay(
      new URL("/v3/get_headers", this.provider).href,
      "POST",
      {
        accept: "application/json",
        "content-type": "application/json",
      },
      new TextEncoder().encode(
        JSON.stringify({ identifier: this.identity.identifier, adi_pb: this.identity.adiPb }),
      ),
    );
    requireSuccess(response.status);
    const headers = record(JSON.parse(text(response.body)));
    if (headers.result !== "Headers") {
      throw new Error("Anisette provider rejected the identity");
    }
    return {
      ...(await this.identityFields()),
      machineID: string(headers["X-Apple-I-MD-M"]),
      oneTimePassword: string(headers["X-Apple-I-MD"]),
      routingInfo: this.identity.routingInfo ?? integer(headers["X-Apple-I-MD-RINFO"]),
      deviceDescription: clientInfo,
      deviceSerialNumber: "0",
      date: new Date(),
      locale: "en_US",
      timeZone: "UTC",
    };
  }
}
