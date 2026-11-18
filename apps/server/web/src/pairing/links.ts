export const pairingHandoffPath = "/pair";

const pairingScheme: string | undefined = import.meta.env.VITE_APPRELAY_APP_SCHEME;
export const pairingConfigured = /^[a-z][a-z0-9+.-]*$/.test(pairingScheme ?? "");

// Match the mobile app's canonical base64url token format.
export function isPairingToken(token: unknown): token is string {
  return typeof token === "string" && /^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$/.test(token);
}

export function pairingAppLink(token: string) {
  return pairingConfigured && isPairingToken(token)
    ? `${pairingScheme}://pair?token=${token}`
    : null;
}
