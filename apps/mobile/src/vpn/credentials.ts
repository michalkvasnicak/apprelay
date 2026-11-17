import { NativeModules } from "react-native";

type CredentialStore = {
  getRegistrationToken(): Promise<string | null>;
  saveRegistrationToken(token: string): Promise<void>;
};

function store(): CredentialStore {
  const native = NativeModules.AppRelayVPN as CredentialStore | undefined;
  if (!native) {
    throw new Error("Device credentials require the native iOS app.");
  }
  return native;
}

export const credentials = {
  get: () => store().getRegistrationToken(),
  save: (token: string) => store().saveRegistrationToken(token),
};
