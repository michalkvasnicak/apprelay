import { accountController } from "../account/controller";
import { validateInspectedEntitlements } from "./entitlements";
import { SigningError, signingLimits, type Inspection } from "./types";
import { SigningWorker } from "./worker-client";

class SigningController {
  private worker?: SigningWorker;
  private inspection?: Inspection;
  private result?: ArrayBuffer;
  private status: "idle" | "inspecting" | "ready" | "signing" | "signed" = "idle";
  private message = "";
  private revision = 0;

  snapshot() {
    return {
      status: this.status,
      message: this.message,
      bundleId: this.inspection?.bundleId,
      resultBytes: this.result?.byteLength,
    };
  }

  reset() {
    this.revision++;
    this.worker?.dispose();
    this.worker = undefined;
    this.inspection = undefined;
    this.result = undefined;
    this.status = "idle";
    this.message = "";
  }

  async inspect(file: File, changed: () => void) {
    this.reset();
    if (!file.size || file.size > signingLimits.inputBytes) {
      throw new SigningError("Choose an IPA smaller than 100 MiB");
    }
    const revision = this.revision;
    const worker = new SigningWorker();
    this.worker = worker;
    this.status = "inspecting";
    changed();
    try {
      const response = await worker.request({ type: "inspect", file }, (message) => {
        this.message = message;
        changed();
      });
      if (revision !== this.revision) {
        return;
      }
      if (response.type !== "inspected") {
        throw new SigningError("The signer returned an unexpected inspection result");
      }
      validateInspectedEntitlements(response.inspection.entitlements, response.inspection.bundleId);
      this.inspection = response.inspection;
      this.status = "ready";
      this.message = "IPA inspected. Basic development signing only.";
    } catch (error) {
      if (revision === this.revision) {
        this.reset();
      }
      throw error;
    } finally {
      changed();
    }
  }

  async sign(changed: () => void) {
    const worker = this.worker;
    const inspection = this.inspection;
    if (this.status !== "ready" || !worker || !inspection) {
      throw new SigningError("Select and inspect an IPA first");
    }
    const revision = this.revision;
    this.status = "signing";
    changed();
    try {
      await accountController.withSigningAssets(inspection.bundleId, async (assets) => {
        const response = await worker.request({ type: "sign", assets }, (message) => {
          this.message = message;
          changed();
        });
        if (revision !== this.revision) {
          return;
        }
        if (response.type !== "signed") {
          throw new SigningError("The signer returned an unexpected signing result");
        }
        // Delivery is intentionally not implemented. No Blob URL, upload, or
        // localStorage persistence: only this private in-memory result boundary.
        this.result = response.bytes;
        this.status = "signed";
        this.message = "Signed result held in memory. Delivery is not configured.";
      });
    } catch (error) {
      if (revision === this.revision) {
        this.reset();
      }
      throw error;
    } finally {
      worker.dispose(); // Release WASM's non-shrinking linear memory.
      if (this.worker === worker) {
        this.worker = undefined;
      }
      changed();
    }
  }
}

export const signingController = new SigningController();
