import { QueryClientProvider } from "@tanstack/react-query";
import { StatusBar } from "expo-status-bar";
import { ActivityIndicator, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import { isVpnAvailable, type VpnStatus } from "./src/vpn/native";
import { queryClient, useVpn } from "./src/vpn/queries";

const statusLabels = {
  disconnected: "Disconnected",
  connecting: "Starting tunnel…",
  connected: "Tunnel active",
  reconnecting: "Reconnecting…",
  disconnecting: "Disconnecting…",
  unknown: "Checking VPN status…",
} as const satisfies Record<VpnStatus, string>;

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <VpnScreen />
    </QueryClientProvider>
  );
}

function VpnScreen() {
  const { identity, status: statusQuery, registration, connection } = useVpn();
  const status = statusQuery.isError ? "unknown" : (statusQuery.data ?? "unknown");
  const active = status === "connected" || status === "connecting" || status === "reconnecting";
  const busy = connection.isPending;
  const disabled =
    !isVpnAvailable ||
    busy ||
    status === "unknown" ||
    status === "disconnecting" ||
    (!active && !identity.data);
  const device = registration.data;
  const errors = [identity.error, statusQuery.error, connection.error].filter(Boolean);
  let buttonLabel = "Connect";
  if (registration.isPending) {
    buttonLabel = "Registering device…";
  } else if (active) {
    buttonLabel = "Disconnect";
  }

  return (
    <View style={styles.screen}>
      <StatusBar style="dark" />
      <ScrollView contentContainerStyle={styles.content}>
        <Text style={styles.title}>AppRelay VPN</Text>
        <Text style={styles.status} accessibilityLiveRegion="polite">
          {isVpnAvailable ? statusLabels[status] : "Native iOS build required"}
        </Text>
        <Text style={styles.description}>
          Connect registers this device securely with the VPN server. Only the returned private
          routes use the tunnel; other destinations use your normal connection. Tunnel status does
          not confirm a successful WireGuard handshake.
        </Text>
        {device && (
          <Text selectable style={styles.description}>
            Device: {device.device_id}
            {"\n"}
            Address: {device.address}
            {"\n"}
            Private routes: {device.wireguard.allowed_ips.join(", ")}
            {"\n"}
            Registration expires: {device.expires_at}
          </Text>
        )}
        {!isVpnAvailable && (
          <Text style={styles.error}>
            Install a signed custom build on an iPhone. Expo Go, Android, web and the iOS simulator
            cannot run this VPN.
          </Text>
        )}
        {errors.length > 0 && (
          <Text accessibilityRole="alert" style={styles.error}>
            {errors.map((error) => error?.message).join("\n")}
          </Text>
        )}
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ disabled, busy }}
          disabled={disabled}
          onPress={() => connection.mutate(active)}
          style={[styles.button, disabled && styles.disabledButton]}
        >
          {busy && <ActivityIndicator color="#fff" />}
          <Text style={styles.buttonText}>{buttonLabel}</Text>
        </Pressable>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: "#f5f7fb" },
  content: { flexGrow: 1, justifyContent: "center", padding: 24, paddingVertical: 72, gap: 16 },
  title: { fontSize: 30, fontWeight: "700", color: "#15243b" },
  status: { fontSize: 20, fontWeight: "600", color: "#2456a6" },
  description: { fontSize: 15, lineHeight: 22, color: "#475569" },
  error: { color: "#b42318", fontSize: 14, lineHeight: 21 },
  button: {
    flexDirection: "row",
    gap: 12,
    justifyContent: "center",
    alignItems: "center",
    backgroundColor: "#2456a6",
    borderRadius: 10,
    minHeight: 52,
    padding: 14,
  },
  disabledButton: { opacity: 0.45 },
  buttonText: { color: "#fff", fontSize: 18, fontWeight: "600" },
});
