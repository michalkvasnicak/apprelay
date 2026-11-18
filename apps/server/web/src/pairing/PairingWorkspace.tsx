import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { ActionError, buttonClass, inputClass } from "../components/forms";
import type { PairedDevice } from "../device";
import { PairingError, type PairingController, type PairingStatus } from "./controller";
import { pairingConfigured } from "./links";
import { TestFlightInvite } from "./TestFlightInvite";

const labels = {
  waiting: "Open AppRelay on your iPhone and confirm pairing.",
  connecting: "Connecting to your iPhone. Keep the VPN connected.",
  pairing: "Enter this code when prompted on your iPhone.",
  done: "Pairing completed successfully.",
  failed: "Pairing failed. You can start a new attempt.",
} as const satisfies Record<PairingStatus, string>;

export function PairingWorkspace({
  controller,
  onPaired,
}: {
  controller: PairingController;
  onPaired: (device: PairedDevice) => void;
}) {
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
      const value = query.state.data?.status;
      if (value === "done" || value === "failed") {
        return false;
      }
      return controller.interval();
    },
  });
  useEffect(() => {
    if (status.data?.status === "done") {
      onPaired(status.data.device);
    }
  }, [status.data, onPaired]);

  const pairing = status.data?.status === "pairing" ? status.data : null;
  const codeExpiresAt = pairing ? Date.parse(pairing.expires_at) : null;
  const [expiredThrough, setExpiredThrough] = useState(0);
  useEffect(() => {
    if (codeExpiresAt === null) {
      return;
    }
    const timer = window.setTimeout(
      () => setExpiredThrough((previous) => Math.max(previous, codeExpiresAt)),
      Math.max(0, codeExpiresAt - Date.now()),
    );
    return () => window.clearTimeout(timer);
  }, [codeExpiresAt]);
  const visibleCode =
    pairing && codeExpiresAt !== null && codeExpiresAt > expiredThrough ? pairing.code : null;
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
      setExpiredThrough(0);
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
  const phase = status.data?.status ?? "waiting";
  const active = generation > 0 && phase !== "done" && phase !== "failed";
  let label: string = labels[phase];
  if (status.data?.status === "failed" && status.data.reason === "timeout") {
    label = "This attempt expired or is no longer available. Start a new attempt.";
  } else if (phase === "pairing" && !visibleCode) {
    label = "Waiting for an updated pairing status.";
  }
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
      <TestFlightInvite />
      <ol className="list-decimal space-y-3 pl-5 text-sm leading-6 text-muted">
        <li>
          <strong className="text-text">Enable Developer Mode.</strong> On your iPhone, open
          Settings → Privacy &amp; Security → Developer Mode. Turn it on, restart your iPhone, then
          unlock it and confirm that you want to turn on Developer Mode.
          <img
            src={`${import.meta.env.BASE_URL}02_privacy_security.webp`}
            alt=""
            width={720}
            height={146}
            className="mt-2 h-auto w-full max-w-sm rounded-lg"
          />
          <img
            src={`${import.meta.env.BASE_URL}01_enable_developer_mode.webp`}
            alt=""
            width={720}
            height={134}
            className="mt-2 h-auto w-full max-w-sm rounded-lg"
          />
        </li>
        <li>
          <strong className="text-text">Keep your iPhone connected to Wi-Fi.</strong> Wi-Fi is
          required throughout pairing and every app upload. Neither works over cellular data.
        </li>
        <li>
          <strong className="text-text">Start pairing on this page.</strong>{" "}
          {onIPhone
            ? "Tap Start pairing, then confirm pairing in AppRelay."
            : "Click Start pairing, then copy the pairing link and open it in Safari on the iPhone with AppRelay installed. Confirm pairing in AppRelay."}{" "}
          Keep the AppRelay VPN connected and leave this page open.
        </li>
        <li>
          <strong className="text-text">Tap Pair with AppRelay.</strong> On your iPhone, return to
          Settings → Privacy &amp; Security → Developer Mode. Tap Pair with AppRelay and enter your
          iPhone passcode if prompted.
          <img
            src={`${import.meta.env.BASE_URL}03_pair_device.webp`}
            alt=""
            width={720}
            height={224}
            className="mt-2 h-auto w-full max-w-sm rounded-lg"
          />
        </li>
        <li>
          <strong className="text-text">Enter the pairing code.</strong> Enter the code shown below
          when your iPhone asks for it. If you enabled notifications in AppRelay, you can also get
          the code from the AppRelay notification.
        </li>
      </ol>
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
        {generation > 0 ? label : "Pairing progress will appear here."}
        {visibleCode && (
          <strong className="mt-1 block font-mono text-2xl tracking-widest">{visibleCode}</strong>
        )}
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
