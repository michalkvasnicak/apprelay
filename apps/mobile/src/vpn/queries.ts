import {
  focusManager,
  QueryClient,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { AppState } from "react-native";

import { registerDevice } from "./config";
import { isVpnAvailable, vpn, type VpnStatus } from "./native";

focusManager.setEventListener((setFocused) => {
  setFocused(AppState.currentState === "active");
  const subscription = AppState.addEventListener("change", (state) =>
    setFocused(state === "active"),
  );
  return () => subscription.remove();
});

export const queryClient = new QueryClient();
const statusKey = ["vpn", "status"] as const;

export function useVpn() {
  const client = useQueryClient();
  const identity = useQuery({
    queryKey: ["vpn", "identity"],
    queryFn: vpn.getIdentity,
    enabled: isVpnAvailable,
    staleTime: Infinity,
    retry: false,
  });
  const status = useQuery({
    queryKey: statusKey,
    queryFn: vpn.getStatus,
    enabled: isVpnAvailable,
    refetchInterval: 1000,
    refetchOnWindowFocus: "always",
    retry: false,
  });
  const registration = useMutation({ mutationFn: registerDevice, retry: false });
  const connection = useMutation({
    mutationFn: async (active: boolean): Promise<VpnStatus> => {
      // Avoid a poll racing the transition written below.
      await client.cancelQueries({ queryKey: statusKey });
      if (active) {
        await vpn.disconnect();
        return "disconnecting";
      }
      if (!identity.data) {
        throw new Error("The device public key is unavailable.");
      }
      let device = registration.data;
      if (!device?.token || Date.parse(device.expires_at) <= Date.now()) {
        device = await registration.mutateAsync(identity.data.publicKey);
      }
      await vpn.connect(device);
      return "connecting";
    },
    onSuccess: (next) => client.setQueryData(statusKey, next),
    onSettled: () => client.invalidateQueries({ queryKey: statusKey }),
    retry: false,
  });
  return { identity, status, registration, connection };
}
