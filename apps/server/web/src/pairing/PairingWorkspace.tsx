import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { ActionError, buttonClass } from "../components/forms";
import type { PairedDevice } from "../device";
import { PairingController, PairingError, type PairingStatus } from "./controller";

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
      const link = controller.link();
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
  const link = controller.link();

  return (
    <section aria-labelledby="pairing-title" className="space-y-4">
      <h2 id="pairing-title" className="text-lg font-semibold">
        Pair your iPhone
      </h2>
      <p className="mt-3 text-sm leading-6 text-muted">
        Continue in Safari on the target iPhone with AppRelay installed. Open AppRelay and confirm
        pairing so we can register this device with your Apple development team before signing. Keep
        its VPN connected until pairing completes.
      </p>
      <div className="mt-5 flex flex-wrap items-center gap-4">
        <button
          type="button"
          className={buttonClass}
          disabled={create.isPending || active}
          onClick={() => create.mutate()}
        >
          {create.isPending ? "Creating attempt…" : "Start pairing"}
        </button>
        {link && (
          <a href={link} rel="noreferrer" className="text-sm underline underline-offset-4">
            Open AppRelay
          </a>
        )}
      </div>
      <p role="status" className="mt-4 text-sm leading-6 text-muted">
        {generation > 0 ? labels[status.data ?? "pending"] : "Pairing progress will appear here."}
      </p>
      <ActionError error={create.error ?? status.error} />
      {active && (
        <p className="mt-2 text-xs leading-5 text-muted">
          If the app did not open, tap Open AppRelay. Return here for the final result.
        </p>
      )}
    </section>
  );
}
