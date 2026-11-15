import { QueryClientProvider } from "@tanstack/react-query";
import { StatusBar } from "expo-status-bar";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import { usePushNotifications } from "./src/notifications/usePushNotifications";
import { usePairing } from "./src/pairing/usePairing";
import { isVpnAvailable, type VpnStatus } from "./src/vpn/native";
import { queryClient, useVpn } from "./src/vpn/queries";

const statusLabels = {
  disconnected: "VPN is off",
  connecting: "Connecting…",
  connected: "VPN connected",
  reconnecting: "Reconnecting…",
  disconnecting: "Disconnecting…",
  unknown: "Checking connection…",
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
  const pairing = usePairing(status, () => registration.mutateAsync());
  const connected = status === "connected";
  const transitioning =
    status === "unknown" ||
    status === "connecting" ||
    status === "reconnecting" ||
    status === "disconnecting";
  const busy = connection.isPending || pairing.start.isPending || transitioning;
  const notifications = usePushNotifications(registration.data, connected && !busy);
  const paired = pairing.available && pairing.start.isSuccess;
  const waitingToPair = pairing.available && connected && !pairing.start.isError && !paired;
  const disabled = !isVpnAvailable || busy || waitingToPair || (!connected && !identity.data);

  let title: string = statusLabels[status];
  let description = connected ? "You're ready to go." : "Connect to use AppRelay.";
  let buttonLabel = connected ? "Disconnect" : "Connect VPN";
  if (pairing.available) {
    title = paired ? "Pairing started" : "Pair with your browser";
    if (paired) {
      description = "Return to your browser to finish. Keep the VPN connected.";
      buttonLabel = "Done";
    } else if (connected) {
      description = pairing.start.isError
        ? "Pairing couldn't start. Try again."
        : "Starting pairing…";
      buttonLabel = "Retry pairing";
    } else {
      description = "Connect the VPN to continue pairing.";
      buttonLabel = "Connect VPN";
    }
  }
  if (!isVpnAvailable) {
    description = "Install AppRelay on an iPhone to connect.";
  }

  let progressLabel: string = statusLabels[status];
  if (pairing.start.isPending || waitingToPair) {
    progressLabel = "Starting pairing…";
  } else if (connection.isPending) {
    progressLabel = connected ? "Disconnecting…" : "Connecting…";
  }

  const error =
    pairing.notice ??
    pairing.start.error?.message ??
    connection.error?.message ??
    identity.error?.message ??
    statusQuery.error?.message ??
    notifications.error;

  function handlePress() {
    if (paired) {
      pairing.dismiss();
    } else if (pairing.available && connected) {
      pairing.start.mutate();
    } else {
      connection.mutate(connected);
    }
  }

  return (
    <View style={styles.screen}>
      <StatusBar style="dark" />
      <View style={styles.content}>
        <Text style={styles.brand}>AppRelay</Text>
        <Text accessibilityLiveRegion="polite" style={styles.title}>
          {title}
        </Text>
        <Text style={styles.description}>{description}</Text>
        {error && (
          <Text accessibilityRole="alert" style={styles.error}>
            {error}
          </Text>
        )}
        {isVpnAvailable && (busy || waitingToPair) ? (
          <View style={styles.progress}>
            <ActivityIndicator color="#2456a6" />
            <Text style={styles.description}>{progressLabel}</Text>
          </View>
        ) : (
          <Pressable
            accessibilityRole="button"
            accessibilityState={{ disabled }}
            disabled={disabled}
            onPress={handlePress}
            style={[styles.button, disabled && styles.disabledButton]}
          >
            <Text style={styles.buttonText}>{buttonLabel}</Text>
          </Pressable>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: "#f5f7fb", justifyContent: "center", padding: 24 },
  content: { width: "100%", maxWidth: 400, alignSelf: "center", gap: 20 },
  brand: { fontSize: 16, fontWeight: "600", color: "#2456a6" },
  title: { fontSize: 30, fontWeight: "700", color: "#15243b" },
  description: { fontSize: 16, lineHeight: 24, color: "#475569" },
  error: { color: "#b42318", fontSize: 14, lineHeight: 21 },
  progress: { flexDirection: "row", alignItems: "center", gap: 12, minHeight: 52 },
  button: {
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#2456a6",
    borderRadius: 12,
    minHeight: 54,
    padding: 16,
  },
  disabledButton: { opacity: 0.45 },
  buttonText: { color: "#fff", fontSize: 18, fontWeight: "600" },
});
