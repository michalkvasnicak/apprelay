import { Anisette } from "./anisette";
import { base64, text, unbase64, utf8 } from "./encoding";
import { developerHeaders } from "./headers";
import { decodePlist, encodePlist, type Plist } from "./plist";
import { relay, requireSuccess } from "./transport";
import type { Session, Team } from "./types";
import { array, integer, record, string } from "./validation";

export class DeveloperPortal {
  constructor(
    private readonly session: Session,
    private readonly anisette: Anisette,
  ) {}

  private check(payload: Record<string, unknown>) {
    if (
      (payload.resultCode !== undefined && integer(payload.resultCode) !== 0) ||
      (payload.errors !== undefined && array(payload.errors).length)
    ) {
      throw new Error("Apple rejected the developer operation; check your account limits");
    }
    return payload;
  }

  async request(path: string, parameters: Record<string, Plist> = {}, teamId?: string) {
    if (this.session.expiresAt !== undefined && this.session.expiresAt <= Date.now()) {
      throw new Error("Apple session expired; sign in again");
    }
    const response = await relay(
      `https://developerservices2.apple.com/services/QH65B2/${path}.action?clientId=XABBG36SBA`,
      "POST",
      developerHeaders(await this.anisette.fresh(), this.session),
      encodePlist({
        clientId: "XABBG36SBA",
        protocolVersion: "QH65B2",
        requestId: crypto.randomUUID().toUpperCase(),
        ...parameters,
        ...(teamId ? { teamId } : {}),
      }),
    );
    requireSuccess(response.status);
    return this.check(decodePlist(response.body));
  }

  async teams(): Promise<Team[]> {
    const payload = await this.request("listTeams");
    return array(payload.teams).map((value) => {
      const team = record(value);
      const type = string(team.type).toLowerCase();
      let kind: Team["kind"] = "unknown";
      if (type === "free") {
        kind = "free";
      } else if (type === "individual" || type === "ind") {
        kind = "individual";
        if (team.memberships !== undefined) {
          const memberships = array(team.memberships).map((entry) => string(record(entry).name));
          if (memberships.length === 1 && memberships[0].toLowerCase().includes("free")) {
            kind = "free";
          }
        }
      } else if (
        ["company/organization", "organization", "company_or_organization"].includes(type)
      ) {
        kind = "organization";
      }
      return { id: string(team.teamId), name: string(team.name), kind };
    });
  }

  async list(path: string, field: string, teamId: string) {
    const result: Record<string, unknown>[] = [];
    for (let pageNumber = 1; pageNumber <= 100; pageNumber++) {
      const payload = await this.request(
        path,
        { pageNumber, pageSize: 500, sort: "name=asc" },
        teamId,
      );
      const page = array(payload[field]).map(record);
      result.push(...page);
      if (page.length < 500) {
        return result;
      }
    }
    throw new Error("Apple collection exceeds the supported limit");
  }

  private async certificateRequest(path: string, method: "GET" | "DELETE", query: URLSearchParams) {
    if (this.session.expiresAt !== undefined && this.session.expiresAt <= Date.now()) {
      throw new Error("Apple session expired; sign in again");
    }
    const response = await relay(
      `https://developerservices2.apple.com/services/v1/certificates${path}`,
      "POST",
      {
        ...developerHeaders(await this.anisette.fresh(), this.session, true),
        "X-HTTP-Method-Override": method,
      },
      utf8(JSON.stringify({ urlEncodedQueryParams: query.toString() })),
    );
    requireSuccess(response.status);
    return response.body;
  }

  async revokeDevelopmentCertificate(teamId: string, certificateId: string) {
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(certificateId)) {
      throw new Error("Invalid Apple certificate ID");
    }
    const body = await this.certificateRequest(
      `/${certificateId}`,
      "DELETE",
      new URLSearchParams({ teamId }),
    );
    if (body.length) {
      this.check(record(JSON.parse(text(body))));
    }
  }

  async certificates(teamId: string) {
    const query = new URLSearchParams({ teamId, "filter[certificateType]": "IOS_DEVELOPMENT" });
    const body = await this.certificateRequest("", "GET", query);
    const payload = this.check(record(JSON.parse(text(body))));
    if (payload.links !== undefined && record(payload.links).next) {
      throw new Error("Paginated certificate collections are not supported yet");
    }
    return array(payload.data).map((value) => {
      const cert = record(value);
      const attributes = cert.attributes === undefined ? cert : record(cert.attributes);
      const content =
        cert.certContent ??
        cert.certificate ??
        attributes.certContent ??
        attributes.certificateContent;
      return {
        id: string(cert.id),
        content:
          content instanceof Uint8Array ? base64(content) : base64(unbase64(string(content))),
        machineName:
          typeof attributes.machineName === "string" ? attributes.machineName : undefined,
      };
    });
  }
}
