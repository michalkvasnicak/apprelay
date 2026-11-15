export interface AnisetteData {
  machineID: string;
  oneTimePassword: string;
  localUserID: string;
  routingInfo: number;
  deviceUniqueIdentifier: string;
  deviceDescription: string;
  deviceSerialNumber: string;
  date: Date;
  locale: string;
  timeZone: string;
}

export interface Session {
  dsid: string;
  authToken: string;
  expiresAt?: number;
}

export interface Team {
  id: string;
  name: string;
  kind: "free" | "individual" | "organization" | "unknown";
}

export interface AnisetteIdentity {
  identifier: string;
  adiPb: string;
  userAgent: string;
  routingInfo?: number;
}

export interface SigningIdentity {
  teamId: string;
  machineId: string;
  privateKeyPem: string;
  csrPem: string;
  submitted: boolean;
  certificate?: string;
}

export interface Profile {
  bundleId: string;
  udid: string;
  data: string;
  expiresAt: number;
}

export interface StoredAccount {
  session: Session;
  anisette: AnisetteIdentity;
  teamId?: string;
  identity?: SigningIdentity;
  profile?: Profile;
}
