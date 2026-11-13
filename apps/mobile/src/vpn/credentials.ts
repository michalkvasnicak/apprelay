import { NativeModules } from "react-native";

type CredentialStore = {
  getRegistrationToken(scope: string): Promise<string | null>;
  saveRegistrationToken(scope: string, token: string): Promise<void>;
};

function store(): CredentialStore {
  const native = NativeModules.AppRelayVPN as CredentialStore | undefined;
  if (!native) {
    throw new Error("Device credentials require the native iOS app.");
  }
  return native;
}

export const credentials = {
  get: (scope: string) => store().getRegistrationToken(scope),
  save: (scope: string, token: string) => store().saveRegistrationToken(scope, token),
};
