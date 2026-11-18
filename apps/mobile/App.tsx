import { QueryClientProvider } from "@tanstack/react-query";
import { StatusBar } from "expo-status-bar";
import { Image, Linking, Pressable, StyleSheet, Text, View } from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";

import { colors } from "./src/colors";
import { type LedState, MacMini } from "./src/MacMini";
import { usePushNotifications } from "./src/notifications/usePushNotifications";
import { usePairing } from "./src/pairing/usePairing";
import { isVpnAvailable, type VpnStatus } from "./src/vpn/native";
import { queryClient, useVpn } from "./src/vpn/queries";

const modelUrl =
  "https://sketchfab.com/3d-models/apple-mac-mini-m4-pro-c0f11a5cfc05448bbefa8c721e5fb0fd";

const statusLabels = {
  disconnected: "Disconnected",
  connecting: "Connecting",
  connected: "Connected",
  reconnecting: "Reconnecting",
  disconnecting: "Disconnecting",
  unknown: "Checking",
} as const satisfies Record<VpnStatus, string>;

export default function App() {
  return (
    <SafeAreaProvider>
      <QueryClientProvider client={queryClient}>
        <VpnScreen />
      </QueryClientProvider>
    </SafeAreaProvider>
  );
}

function VpnScreen() {
  const { identity, status: statusQuery, registration, connection } = useVpn();
  const status = statusQuery.isError ? "unknown" : (statusQuery.data ?? "unknown");
  usePairing(registration.mutateAsync);
  const connected = status === "connected";
  const transitioning =
    status === "unknown" ||
    status === "connecting" ||
    status === "reconnecting" ||
    status === "disconnecting";
  const busy = connection.isPending || transitioning;
  const notifications = usePushNotifications(registration.data, connected && !busy);
  const loading = isVpnAvailable && busy;
  const disabled = !isVpnAvailable || loading || (!connected && !identity.data);
  const buttonLabel = connected ? "Disconnect" : "Connect";

  let led: LedState = "off";
  if (loading) {
    led = "pulsing";
  } else if (connected) {
    led = "on";
  }

  const statusLabel = isVpnAvailable ? statusLabels[status] : "Unavailable";

  const error =
    connection.error?.message ??
    identity.error?.message ??
    statusQuery.error?.message ??
    notifications.error;

  return (
    <SafeAreaView style={styles.screen}>
      <StatusBar style="light" />
      <View style={styles.content}>
        <View style={styles.brandRow}>
          {/* PNG rasterized from apps/server/web/public/logo.svg for native Image. */}
          <Image source={require("./assets/logo.png")} accessible={false} style={styles.logo} />
          <Text style={styles.brand}>AppRelay</Text>
        </View>

        <View style={styles.center}>
          <View style={styles.model}>
            <MacMini led={led} />
          </View>
          <Text accessibilityLiveRegion="polite" style={styles.status}>
            {statusLabel}
          </Text>
          {error && (
            <Text accessibilityRole="alert" style={styles.error}>
              {error}
            </Text>
          )}
        </View>

        <Pressable
          accessibilityRole="button"
          accessibilityState={{ disabled }}
          disabled={disabled}
          onPress={() => connection.mutate(connected)}
          style={({ pressed }) => [
            styles.button,
            connected && styles.outlineButton,
            disabled && styles.disabledButton,
            pressed && styles.pressedButton,
          ]}
        >
          <Text style={[styles.buttonText, connected && styles.outlineButtonText]}>
            {buttonLabel}
          </Text>
        </Pressable>
        <Text
          accessibilityRole="link"
          onPress={() => Linking.openURL(modelUrl)}
          style={styles.credit}
        >
          Model: “Apple Mac mini M4 pro” by mistjs · CC BY-NC 4.0
        </Text>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.background },
  content: { flex: 1, paddingHorizontal: 24, paddingTop: 24, paddingBottom: 24 },
  brandRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  logo: { width: 32, height: 32 },
  brand: { fontSize: 18, fontWeight: "700", color: colors.text },
  center: { flex: 1, alignItems: "center", justifyContent: "center", gap: 28 },
  model: { width: "100%", height: 280 },
  status: { fontSize: 28, fontWeight: "700", letterSpacing: -0.5, color: colors.text },
  error: {
    color: colors.error,
    backgroundColor: colors.errorSurface,
    borderRadius: 12,
    overflow: "hidden",
    paddingHorizontal: 14,
    paddingVertical: 10,
    fontSize: 14,
    lineHeight: 20,
    textAlign: "center",
  },
  button: {
    alignItems: "center",
    justifyContent: "center",
    minHeight: 60,
    borderRadius: 30,
    borderWidth: 1,
    borderColor: colors.primary,
    backgroundColor: colors.primary,
  },
  outlineButton: { backgroundColor: "transparent", borderColor: colors.border },
  disabledButton: { opacity: 0.4 },
  pressedButton: { opacity: 0.8 },
  credit: { marginTop: 16, fontSize: 12, textAlign: "center", color: colors.muted },
  buttonText: { color: colors.background, fontSize: 17, fontWeight: "700" },
  outlineButtonText: { color: colors.text },
});
