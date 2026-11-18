import { useMutation, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import {
  Alert,
  Pending,
  Spinner,
  Steps,
  buttonClass,
  inputClass,
  secondaryClass,
} from "../components/forms";
import type { PairedDevice } from "../device";
import { PairingError, type PairingController, type PairingStatus } from "./controller";
import { pairingConfigured } from "./links";
import { TestFlightInvite } from "./TestFlightInvite";

const labels = {
  waiting: "Open AppRelay on your iPhone and confirm pairing.",
  connecting: "Connecting to your iPhone…",
  pairing: "Enter this code on your iPhone.",
  done: "Paired.",
  failed: "Pairing failed.",
} as const satisfies Record<PairingStatus, string>;

const base = import.meta.env.BASE_URL;

function Guide() {
  return (
    <details className="group rounded-xl border border-text/10 bg-background/40 text-sm">
      <summary className="flex cursor-pointer list-none items-center justify-between px-4 py-3 font-medium">
        Setup checklist
        <span aria-hidden="true" className="text-muted transition group-open:rotate-180">
          ⌄
        </span>
      </summary>
      <div className="space-y-4 border-t border-text/10 px-4 py-4 text-muted">
        <ul className="list-disc space-y-1.5 pl-4">
          <li>
            Developer Mode on: Settings → Privacy &amp; Security → Developer Mode, then restart.
          </li>
          <li>iPhone on Wi-Fi (not cellular) with the AppRelay VPN connected.</li>
          <li>
            In Developer Mode settings, tap{" "}
            <strong className="text-text">Pair with AppRelay</strong> when the code is shown below.
          </li>
        </ul>
        <div className="grid gap-2">
          {(
            [
              ["02_privacy_security.webp", 146],
              ["01_enable_developer_mode.webp", 134],
              ["03_pair_device.webp", 224],
            ] as const
          ).map(([name, height]) => (
            <img
              key={name}
              src={`${base}${name}`}
              alt=""
              width={720}
              height={height}
              loading="lazy"
              className="h-auto w-full max-w-sm rounded-lg"
            />
          ))}
        </div>
        <TestFlightInvite />
      </div>
    </details>
  );
}

export function PairingWorkspace({
  controller,
  onPaired,
}: {
  controller: PairingController;
  onPaired: (device: PairedDevice) => void;
}) {
  const [generation, setGeneration] = useState(0);
  const [started, setStarted] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "manual">("idle");
  const onIPhone = /\biPhone\b/i.test(navigator.userAgent);
  const status = useQuery({
    queryKey: ["browser-pairing", generation],
    queryFn: ({ signal }) => controller.status(signal),
    enabled: started,
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
      setStarted(true);
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

  function cancel() {
    controller.reset();
    create.reset();
    setStarted(false);
    setCopyState("idle");
    setExpiredThrough(0);
  }

  const phase = status.data?.status ?? "waiting";
  const failed = started && phase === "failed";
  const active = started && phase !== "done" && !failed;
  let label: string = labels[phase];
  if (failed && status.data?.status === "failed" && status.data.reason === "timeout") {
    label = "This attempt expired.";
  } else if (phase === "pairing" && !visibleCode) {
    label = "Waiting for a new code…";
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
    <section aria-labelledby="pairing-title" className="space-y-5">
      <Steps current={1} />
      <h2 id="pairing-title" className="text-xl font-semibold tracking-tight">
        Pair your iPhone
      </h2>
      {!pairingConfigured && <Alert error="Pairing is unavailable in this build." />}
      {!active && !failed && <Guide />}

      {!started && (
        <button
          type="button"
          className={buttonClass + " w-full"}
          disabled={!pairingConfigured || create.isPending}
          onClick={() => create.mutate()}
        >
          <Pending active={create.isPending} label="Creating attempt…">
            Start pairing
          </Pending>
        </button>
      )}

      {started && (
        <div
          role="status"
          aria-live="polite"
          className="space-y-4 rounded-2xl border border-text/10 bg-background/50 p-5 text-center"
        >
          <p className="flex items-center justify-center gap-2.5 text-sm text-muted">
            {active && <Spinner className="size-4 text-primary" />}
            {label}
          </p>
          {visibleCode && (
            <strong className="block font-mono text-4xl tracking-[0.3em] text-primary">
              {visibleCode}
            </strong>
          )}
        </div>
      )}

      {active && (
        <div className="flex flex-wrap gap-3">
          {appLink && (
            <a href={appLink} rel="noreferrer" className={buttonClass + " flex-1"}>
              Open AppRelay
            </a>
          )}
          {handoffLink && (
            <button
              type="button"
              className={secondaryClass + " flex-1"}
              onClick={() => void copyPairingLink()}
            >
              {copyState === "copied" ? "Link copied ✓" : "Copy link for iPhone"}
            </button>
          )}
          <button type="button" className={secondaryClass} onClick={cancel}>
            Cancel
          </button>
        </div>
      )}
      {handoffLink && copyState === "manual" && (
        <label className="block text-sm text-muted">
          Copy this link manually
          <input
            type="text"
            readOnly
            value={handoffLink}
            onFocus={(event) => event.currentTarget.select()}
            className={inputClass}
          />
        </label>
      )}

      {failed && (
        <div className="space-y-3">
          <div className="flex gap-3">
            <button
              type="button"
              className={buttonClass + " flex-1"}
              disabled={create.isPending}
              onClick={() => create.mutate()}
            >
              <Pending active={create.isPending} label="Creating attempt…">
                Try again
              </Pending>
            </button>
            <button type="button" className={secondaryClass} onClick={cancel}>
              Back
            </button>
          </div>
        </div>
      )}

      <Alert error={create.error} onDismiss={create.error ? () => create.reset() : undefined} />
      {status.error && active && (
        <p role="alert" className="text-sm text-muted">
          {status.error.message} Retrying…
        </p>
      )}
    </section>
  );
}
