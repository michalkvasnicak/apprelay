import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";

import { useAccountBusy } from "../account/hooks";
import { secondaryClass } from "../components/forms";
import { clearTargetDevice, type PairedDevice } from "../device";
import { PairingWorkspace } from "../pairing/PairingWorkspace";
import { SigningPanel } from "./SigningPanel";
import { signingController } from "./controller";

export function SigningWorkspace({
  file,
  onFileSelected,
}: {
  file?: File;
  onFileSelected: (file: File) => void;
}) {
  const [device, setDevice] = useState<PairedDevice | null>(null);
  const busy = useAccountBusy();
  const client = useQueryClient();
  const onPaired = useCallback((paired: PairedDevice) => setDevice(paired), []);
  useEffect(() => clearTargetDevice, []);

  if (!device) {
    return <PairingWorkspace onPaired={onPaired} />;
  }

  return (
    <div className="space-y-6">
      <SigningPanel file={file} device={device} onFileSelected={onFileSelected} />
      <button
        type="button"
        className={secondaryClass}
        disabled={busy}
        onClick={() => {
          clearTargetDevice();
          signingController.reset();
          void client.invalidateQueries({ queryKey: ["signing"] });
          setDevice(null);
        }}
      >
        Pair again or choose another iPhone
      </button>
    </div>
  );
}
