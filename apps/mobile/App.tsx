import { QueryClientProvider } from "@tanstack/react-query";
import { StatusBar } from "expo-status-bar";
import {
  ActivityIndicator,
  Image,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";

import { colors } from "./src/colors";
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
    title = "VPN unavailable";
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

  const secondaryAction = connected && !pairing.available;

  return (
    <View style={styles.screen}>
      <StatusBar style="light" />
      <ScrollView contentContainerStyle={styles.scrollContent}>
        <View style={styles.content}>
          <View style={styles.header}>
            <View style={styles.brandRow}>
              {/* PNG rasterized from apps/server/web/public/logo.svg for native Image. */}
              <Image source={require("./assets/logo.png")} accessible={false} style={styles.logo} />
              <Text style={styles.brand}>AppRelay</Text>
            </View>
            <Text accessibilityRole="header" style={styles.heading}>
              VPN connection
            </Text>
          </View>

          <View style={styles.card}>
            <View style={styles.statusBadge}>
              <View
                style={[styles.statusDot, isVpnAvailable && connected && styles.connectedDot]}
              />
              <Text style={styles.statusLabel}>
                {pairing.available && isVpnAvailable ? "Browser pairing" : "VPN status"}
              </Text>
            </View>
            <View style={styles.summary}>
              <Text accessibilityLiveRegion="polite" style={styles.title}>
                {title}
              </Text>
              <Text style={styles.description}>{description}</Text>
            </View>
            {error && (
              <View style={styles.errorPanel}>
                <Text accessibilityRole="alert" style={styles.error}>
                  {error}
                </Text>
              </View>
            )}
            {isVpnAvailable && (busy || waitingToPair) ? (
              <View style={styles.progress}>
                <ActivityIndicator color={colors.primary} />
                <Text accessibilityLiveRegion="polite" style={styles.progressText}>
                  {progressLabel}
                </Text>
              </View>
            ) : (
              <Pressable
                accessibilityRole="button"
                accessibilityState={{ disabled }}
                disabled={disabled}
                onPress={handlePress}
                style={({ pressed }) => [
                  styles.button,
                  secondaryAction && styles.secondaryButton,
                  disabled && styles.disabledButton,
                  pressed && styles.pressedButton,
                ]}
              >
                <Text
                  style={[
                    styles.buttonText,
                    (secondaryAction || disabled) && styles.secondaryButtonText,
                  ]}
                >
                  {buttonLabel}
                </Text>
              </Pressable>
            )}
          </View>
        </View>
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.background },
  scrollContent: {
    flexGrow: 1,
    justifyContent: "center",
    paddingHorizontal: 24,
    paddingVertical: 64,
  },
  content: { width: "100%", maxWidth: 440, alignSelf: "center", gap: 24 },
  header: { gap: 16 },
  brandRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  logo: { width: 48, height: 48 },
  brand: { flexShrink: 1, fontSize: 18, fontWeight: "600", color: colors.primary },
  heading: { fontSize: 32, fontWeight: "700", letterSpacing: -0.8, color: colors.text },
  card: {
    padding: 24,
    gap: 20,
    borderRadius: 20,
    borderWidth: 1,
    borderColor: colors.border,
    backgroundColor: colors.surface,
  },
  statusBadge: {
    alignSelf: "flex-start",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderRadius: 16,
    backgroundColor: colors.surfaceRaised,
  },
  statusDot: { width: 8, height: 8, borderRadius: 4, backgroundColor: colors.muted },
  connectedDot: { backgroundColor: colors.primary },
  statusLabel: { flexShrink: 1, fontSize: 13, fontWeight: "600", color: colors.text },
  summary: { gap: 6 },
  title: { fontSize: 24, fontWeight: "700", letterSpacing: -0.4, color: colors.text },
  description: { fontSize: 16, lineHeight: 24, color: colors.muted },
  errorPanel: { backgroundColor: colors.errorSurface, borderRadius: 12, padding: 12 },
  error: { color: colors.error, fontSize: 14, lineHeight: 21 },
  progress: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 10,
    minHeight: 52,
    padding: 12,
    borderRadius: 12,
    backgroundColor: colors.surfaceRaised,
  },
  progressText: { flexShrink: 1, color: colors.muted, fontSize: 15 },
  button: {
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: colors.primary,
    borderColor: colors.primary,
    borderWidth: 1,
    borderRadius: 12,
    minHeight: 52,
    padding: 12,
  },
  secondaryButton: { backgroundColor: colors.surfaceRaised, borderColor: colors.border },
  disabledButton: { backgroundColor: colors.surface, borderColor: colors.border, opacity: 0.6 },
  pressedButton: { opacity: 0.8 },
  buttonText: { color: colors.background, fontSize: 16, fontWeight: "700", textAlign: "center" },
  secondaryButtonText: { color: colors.text },
});
