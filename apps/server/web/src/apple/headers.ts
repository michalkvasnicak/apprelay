import type { AnisetteData, Session } from "./types";
import { base64, utf8 } from "./encoding";

/** Application identifier used in GrandSlam client parameters and portal headers. */
export const xcodeAppId = "com.apple.gs.xcode.auth";
/** SideStore's default developer/2FA version at the pinned SideSign integration. */
const xcodeVersion = "26.0 (26A242)";
/** SideSign's default GrandSlam AuthKit fingerprint. */
const grandSlamUserAgent = "AuthKit/1 (Macintosh; OS X 26.6) (com.apple.dt.Xcode/26.0)";
/** Media types owned by the Apple wire policy, not the transport. */
const mediaTypes = {
  plist: "text/x-xml-plist",
  buddy: "application/x-buddyml",
  phone: "application/x-plist",
  developerJson: "application/vnd.api+json",
};
/** Fixed developer application headers, independent of anisette identity. */
const developerApplicationHeaders = {
  "User-Agent": "Xcode",
  "Accept-Language": "en-us",
  "X-Apple-App-Info": xcodeAppId,
  "X-Xcode-Version": xcodeVersion,
};
/** Remove fractional seconds from Apple client timestamps. */
const fractionalSeconds = /\.\d{3}Z$/;
/** GrandSlam bootstrap flags; cpd does not serialize the client description. */
const fixedClientParameters = {
  bootstrap: true,
  icscrec: true,
  pbe: false,
  prkgen: true,
  svct: "iCloud",
};

export function clientData(data: AnisetteData) {
  return {
    "X-Apple-I-Client-Time": data.date.toISOString().replace(fractionalSeconds, "Z"),
    "X-Apple-Locale": data.locale,
    "X-Apple-I-TimeZone": data.timeZone,
    "X-Apple-I-MD": data.oneTimePassword,
    "X-Apple-I-MD-LU": data.localUserID,
    "X-Apple-I-MD-M": data.machineID,
    "X-Apple-I-MD-RINFO": data.routingInfo,
    "X-Mme-Device-Id": data.deviceUniqueIdentifier,
    "X-Apple-I-SRL-NO": data.deviceSerialNumber,
  };
}
export function clientParameters(data: AnisetteData) {
  return { ...fixedClientParameters, loc: data.locale, ...clientData(data) };
}
function identityHeaders(data: AnisetteData): Record<string, string> {
  return {
    ...Object.fromEntries(
      Object.entries(clientData(data)).map(([key, value]) => [key, String(value)]),
    ),
    "X-MMe-Client-Info": data.deviceDescription,
  };
}
export function grandSlamHeaders(data: AnisetteData) {
  return {
    "Content-Type": mediaTypes.plist,
    Accept: "*/*",
    "User-Agent": grandSlamUserAgent,
    "X-MMe-Client-Info": data.deviceDescription,
  };
}
export function developerHeaders(data: AnisetteData, session: Session, json = false) {
  const headers = identityHeaders(data);
  delete headers["X-Apple-I-SRL-NO"];
  return {
    ...headers,
    ...developerApplicationHeaders,
    "Content-Type": json ? mediaTypes.developerJson : mediaTypes.plist,
    Accept: json ? mediaTypes.developerJson : mediaTypes.plist,
    "X-Apple-I-Locale": data.locale,
    "X-Apple-I-Identity-Id": session.dsid,
    "X-Apple-GS-Token": session.authToken,
  };
}
export function twoFactorHeaders(data: AnisetteData, dsid: string, token: string) {
  return {
    ...identityHeaders(data),
    ...developerApplicationHeaders,
    "Content-Type": mediaTypes.phone,
    Accept: mediaTypes.buddy,
    "X-Apple-Identity-Token": base64(utf8(`${dsid}:${token}`)),
  };
}
