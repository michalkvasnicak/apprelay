import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";

import { useAccountBusy } from "../account/hooks";
import { secondaryClass } from "../components/forms";
import { clearTargetDevice, type PairedDevice } from "../device";
import { PairingWorkspace } from "../pairing/PairingWorkspace";
import { PairingController } from "../pairing/controller";
import { SigningPanel } from "./SigningPanel";
import { signingController } from "./controller";

export function SigningWorkspace({
  file,
  onFileSelected,
}: {
  file?: File;
  onFileSelected: (file: File) => void;
}) {
  const [pairing, setPairing] = useState(() => new PairingController());
  const [device, setDevice] = useState<PairedDevice | null>(null);
  const busy = useAccountBusy();
  const client = useQueryClient();
  const onPaired = useCallback((paired: PairedDevice) => setDevice(paired), []);
  useEffect(() => clearTargetDevice, []);

  if (!device) {
    return <PairingWorkspace controller={pairing} onPaired={onPaired} />;
  }

  return (
    <div className="space-y-5">
      <SigningPanel file={file} device={device} pairing={pairing} onFileSelected={onFileSelected} />
      <button
        type="button"
        className={secondaryClass}
        disabled={busy}
        onClick={() => {
          clearTargetDevice();
          signingController.reset();
          void client.invalidateQueries({ queryKey: ["signing"] });
          setPairing(new PairingController());
          setDevice(null);
        }}
      >
        Change iPhone
      </button>
    </div>
  );
}
