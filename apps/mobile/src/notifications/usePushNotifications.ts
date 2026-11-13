import * as Notifications from "expo-notifications";
import { useEffect, useState } from "react";
import { AppState } from "react-native";

import type { DeviceRegistration } from "../vpn/config";
import { isVpnAvailable, vpn } from "../vpn/native";
import { syncPushToken } from "./api";

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldShowBanner: true,
    shouldShowList: true,
    shouldPlaySound: true,
    shouldSetBadge: false,
  }),
});

function apnsToken(token: Notifications.DevicePushToken): string {
  if (token.type !== "ios" || !/^(?:[a-fA-F0-9]{2})+$/.test(token.data)) {
    throw new Error("The device did not return a valid hexadecimal APNs token.");
  }
  return token.data.toLowerCase();
}

async function hasNotificationPermission(): Promise<boolean> {
  let permission = await Notifications.getPermissionsAsync();
  if (
    permission.ios?.status === Notifications.IosAuthorizationStatus.NOT_DETERMINED &&
    permission.canAskAgain
  ) {
    permission = await Notifications.requestPermissionsAsync({
      ios: { allowAlert: true, allowSound: true, allowBadge: true },
    });
  }
  return (
    permission.granted ||
    permission.ios?.status === Notifications.IosAuthorizationStatus.PROVISIONAL ||
    permission.ios?.status === Notifications.IosAuthorizationStatus.EPHEMERAL
  );
}

export function usePushNotifications(device: DeviceRegistration | undefined, connected: boolean) {
  const [error, setError] = useState<string | null>(null);
  const deviceId = device?.device_id;
  const expiresAt = device?.expires_at;

  useEffect(() => {
    if (!isVpnAvailable || !connected || !deviceId || !expiresAt) {
      return;
    }

    const registrationId = deviceId;
    const expiration = Date.parse(expiresAt);
    const controller = new AbortController();
    let queue = Promise.resolve();
    let syncedToken: string | null | undefined;

    // Serialize permission prompts and writes so a rolled token cannot be
    // overwritten by an older in-flight registration.
    function enqueue(token?: Notifications.DevicePushToken, force = false) {
      queue = queue.then(async () => {
        if (
          controller.signal.aborted ||
          AppState.currentState !== "active" ||
          expiration <= Date.now()
        ) {
          return;
        }
        try {
          // Recheck native state: the cached UI status may be stale on foregrounding.
          if ((await vpn.getStatus()) !== "connected" || controller.signal.aborted) {
            return;
          }
          setError(null);
          const allowed = await hasNotificationPermission();
          if (controller.signal.aborted) {
            return;
          }
          const nextToken = allowed
            ? apnsToken(token ?? (await Notifications.getDevicePushTokenAsync()))
            : null;
          if (controller.signal.aborted) {
            return;
          }
          if (force || nextToken !== syncedToken) {
            if ((await vpn.getStatus()) !== "connected" || controller.signal.aborted) {
              return;
            }
            await syncPushToken(registrationId, nextToken, controller.signal);
            syncedToken = nextToken;
          }
        } catch (cause) {
          if (!controller.signal.aborted) {
            setError(cause instanceof Error ? cause.message : "Push token sync failed.");
          }
        }
      });
    }

    // Never fetch a token directly inside its listener: native registration
    // itself emits this event. The queued event carries the token instead.
    const tokenSubscription = Notifications.addPushTokenListener((token) => enqueue(token));
    const appStateSubscription = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        enqueue(undefined, true);
      }
    });
    enqueue(undefined, true);

    return () => {
      controller.abort();
      tokenSubscription.remove();
      appStateSubscription.remove();
    };
  }, [deviceId, expiresAt, connected]);

  return { error };
}
