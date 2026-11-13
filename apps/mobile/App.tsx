import { StatusBar } from "expo-status-bar";
import { useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";

import { getVpnServerConfig } from "./src/vpn/config";
import { isVpnAvailable, vpn, type VpnStatus } from "./src/vpn/native";

function message(error: unknown): string {
  return error instanceof Error ? error.message : "An unexpected VPN error occurred.";
}

const statusLabels = {
  disconnected: "Disconnected",
  connecting: "Starting tunnel…",
  connected: "Tunnel active",
  reconnecting: "Reconnecting…",
  disconnecting: "Disconnecting…",
  unknown: "Checking VPN status…",
} as const satisfies Record<VpnStatus, string>;

export default function App() {
  const configuration = useMemo(() => {
    try {
      return { server: getVpnServerConfig(), error: null };
    } catch (error) {
      return { server: null, error: message(error) };
    }
  }, []);
  const [status, setStatus] = useState<VpnStatus>("unknown");
  const [publicKey, setPublicKey] = useState("");
  const [clientAddress, setClientAddress] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);

  useEffect(() => {
    if (!isVpnAvailable) {
      return;
    }
    let mounted = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing || AppState.currentState !== "active") {
        return;
      }
      refreshing = true;
      try {
        const current = await vpn.getStatus();
        if (mounted) {
          setStatus(current);
          setStatusError(null);
        }
      } catch (cause) {
        if (mounted) {
          setStatus("unknown");
          setStatusError(message(cause));
        }
      } finally {
        refreshing = false;
      }
    };
    void vpn
      .getIdentity()
      .then((identity) => {
        if (mounted) {
          setPublicKey(identity.publicKey);
          setClientAddress(identity.clientAddress);
        }
      })
      .catch((cause: unknown) => {
        if (mounted) {
          setError(message(cause));
        }
      });
    void refresh();
    const interval = setInterval(() => {
      void refresh();
    }, 1000);
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        void refresh();
      }
    });
    return () => {
      mounted = false;
      clearInterval(interval);
      subscription.remove();
    };
  }, []);

  const active = status === "connected" || status === "connecting" || status === "reconnecting";
  const disabled =
    !isVpnAvailable ||
    busy ||
    status === "unknown" ||
    status === "disconnecting" ||
    (!active && (!configuration.server || !publicKey || !clientAddress.trim()));

  async function toggleVpn() {
    if (disabled) {
      return;
    }
    setBusy(true);
    setError(null);
    try {
      if (active) {
        await vpn.disconnect();
        setStatus("disconnecting");
      } else if (configuration.server) {
        await vpn.connect(configuration.server, clientAddress);
        setStatus("connecting");
      }
    } catch (cause) {
      setError(message(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <View style={styles.screen}>
      <StatusBar style="dark" />
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text style={styles.title}>AppRelay VPN</Text>
        <Text style={styles.status} accessibilityLiveRegion="polite">
          {isVpnAvailable ? statusLabels[status] : "Native iOS build required"}
        </Text>
        {configuration.server && (
          <Text style={styles.description}>
            Server: {configuration.server.endpoint.host}:{configuration.server.endpoint.port}
            {"\n"}
            Private route: {configuration.server.allowedIPs[0]} only
          </Text>
        )}
        <Text style={styles.description}>
          Other destinations use your normal connection. An active tunnel does not confirm that the
          server has accepted this device.
        </Text>
        <Text style={styles.label}>Device VPN address</Text>
        <TextInput
          accessibilityLabel="Device VPN address"
          style={styles.input}
          value={clientAddress}
          onChangeText={setClientAddress}
          placeholder="e.g. 10.77.0.2"
          autoCapitalize="none"
          autoCorrect={false}
          keyboardType="numbers-and-punctuation"
          editable={isVpnAvailable && !active && !busy && status === "disconnected"}
        />
        <Text style={styles.description}>
          Assign an unused address and register it with this device’s public key on the Mac before
          connecting. Do not reuse an address across devices.
        </Text>
        <Text style={styles.label}>Device public key</Text>
        <Text selectable style={styles.key}>
          {publicKey || "Unavailable until the native module initializes."}
        </Text>
        {!isVpnAvailable && (
          <Text style={styles.error}>
            Install a signed custom build on an iPhone. Expo Go, Android, web and the iOS simulator
            cannot run this VPN.
          </Text>
        )}
        {(configuration.error || error || statusError) && (
          <Text accessibilityRole="alert" style={styles.error}>
            {[configuration.error, error, statusError].filter(Boolean).join("\n")}
          </Text>
        )}
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ disabled, busy }}
          disabled={disabled}
          onPress={() => {
            void toggleVpn();
          }}
          style={[styles.button, disabled && styles.disabledButton]}
        >
          {busy && <ActivityIndicator color="#fff" />}
          <Text style={styles.buttonText}>{active ? "Disconnect" : "Connect"}</Text>
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
  label: { fontSize: 15, fontWeight: "600", color: "#15243b" },
  input: {
    borderWidth: 1,
    borderColor: "#94a3b8",
    borderRadius: 8,
    padding: 14,
    fontSize: 17,
    backgroundColor: "#fff",
    color: "#15243b",
  },
  key: { fontSize: 14, fontFamily: "monospace", color: "#15243b" },
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
