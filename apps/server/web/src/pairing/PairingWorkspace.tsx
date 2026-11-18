import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { ActionError, buttonClass, inputClass } from "../components/forms";
import type { PairedDevice } from "../device";
import { PairingController, PairingError, type PairingStatus } from "./controller";
import { pairingConfigured } from "./links";
import { TestFlightInvite } from "./TestFlightInvite";

const labels = {
  pending: "Open AppRelay on this iPhone and confirm pairing.",
  pairing: "Pairing is in progress. Keep the VPN connected and follow any iPhone prompts.",
  success: "Pairing completed successfully.",
  failed: "Pairing failed. You can start a new attempt.",
  expired: "This attempt expired or is no longer available. Start a new attempt.",
} as const satisfies Record<PairingStatus, string>;

export function PairingWorkspace({ onPaired }: { onPaired: (device: PairedDevice) => void }) {
  const [controller] = useState(() => new PairingController());
  const [generation, setGeneration] = useState(0);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "manual">("idle");
  const onIPhone = /\biPhone\b/i.test(navigator.userAgent);
  const status = useQuery({
    queryKey: ["browser-pairing", generation],
    queryFn: ({ signal }) => controller.status(signal),
    enabled: generation > 0,
    retry: false,
    gcTime: 0,
    refetchOnWindowFocus: "always",
    refetchInterval: (query) => {
      const value = query.state.data;
      if (value && value !== "pending" && value !== "pairing") {
        return false;
      }
      return controller.interval();
    },
  });
  useEffect(() => {
    if (status.data === "success") {
      const device = controller.device();
      if (device) {
        onPaired(device);
      }
    }
  }, [status.data, onPaired, controller]);
  const create = useMutation({
    mutationFn: async () => {
      try {
        await controller.create();
      } catch (error) {
        throw new PairingError(
          error instanceof PairingError ? error.message : "Could not create a pairing attempt.",
        );
      }
    },
    retry: false,
    gcTime: 0,
    onSuccess: () => {
      setGeneration((value) => value + 1);
      setCopyState("idle");
      const link = onIPhone ? controller.appLink() : null;
      if (link) {
        // Safari may block navigation after an asynchronous request. The real anchor below
        // lets the user reopen the same attempt without creating another one.
        try {
          window.location.assign(link);
        } catch {
          /* Use the visible fallback link. */
        }
      }
    },
  });
  const active =
    generation > 0 && (!status.data || status.data === "pending" || status.data === "pairing");
  const appLink = onIPhone ? controller.appLink() : null;
  const handoffLink = onIPhone ? null : controller.handoffLink();

  async function copyPairingLink() {
    const link = controller.handoffLink();
    if (!link) {
      return;
    }
    try {
      await navigator.clipboard.writeText(link);
      setCopyState("copied");
    } catch {
      setCopyState("manual");
    }
  }

  return (
    <section aria-labelledby="pairing-title" className="space-y-4">
      <h2 id="pairing-title" className="text-lg font-semibold">
        Pair your iPhone
      </h2>
      {!pairingConfigured && (
        <p role="alert" className="text-sm text-error">
          Pairing is unavailable. Set VITE_APPRELAY_APP_SCHEME for this web build.
        </p>
      )}
      <p className="mt-3 text-sm leading-6 text-muted">
        {onIPhone
          ? "Open AppRelay on this iPhone and confirm pairing. Keep its VPN connected until pairing completes."
          : "Start pairing here, then copy the pairing link and open it in Safari on the target iPhone with AppRelay installed. Keep this page open and return here after pairing."}
      </p>
      <TestFlightInvite />
      <div className="mt-5 flex flex-wrap items-center gap-4">
        <button
          type="button"
          className={buttonClass}
          disabled={!pairingConfigured || create.isPending || active}
          onClick={() => create.mutate()}
        >
          {create.isPending ? "Creating attempt…" : "Start pairing"}
        </button>
        {appLink && (
          <a href={appLink} rel="noreferrer" className="text-sm underline underline-offset-4">
            Open AppRelay
          </a>
        )}
        {handoffLink && (
          <button type="button" className={buttonClass} onClick={() => void copyPairingLink()}>
            Copy pairing link
          </button>
        )}
      </div>
      {handoffLink && copyState === "copied" && (
        <p role="status" className="text-sm text-muted">
          Link copied. Open it in Safari on the target iPhone before it expires.
        </p>
      )}
      {handoffLink && copyState === "manual" && (
        <label className="block text-sm text-muted">
          Clipboard unavailable. Copy this pairing link manually:
          <input
            type="text"
            readOnly
            value={handoffLink}
            onFocus={(event) => event.currentTarget.select()}
            className={inputClass}
          />
        </label>
      )}
      <p role="status" className="mt-4 text-sm leading-6 text-muted">
        {generation > 0 ? labels[status.data ?? "pending"] : "Pairing progress will appear here."}
      </p>
      <ActionError error={create.error ?? status.error} />
      {active && appLink && (
        <p className="mt-2 text-xs leading-5 text-muted">
          If the app did not open, tap Open AppRelay. Return here for the final result.
        </p>
      )}
    </section>
  );
}
