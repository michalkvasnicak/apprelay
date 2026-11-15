import { SigningError, type WorkerRequest, type WorkerResponse } from "./types";

export class SigningWorker {
  private readonly worker = new Worker(new URL("./signer.worker.ts", import.meta.url), {
    type: "module",
  });
  private reject?: (error: SigningError) => void;
  private timer?: ReturnType<typeof setTimeout>;

  request(request: WorkerRequest, onProgress: (message: string) => void): Promise<WorkerResponse> {
    if (this.reject) {
      return Promise.reject(new SigningError("A signing operation is already running"));
    }
    return new Promise((resolve, reject) => {
      const finish = () => {
        clearTimeout(this.timer);
        this.reject = undefined;
        this.worker.onmessage = null;
        this.worker.onerror = null;
        this.worker.onmessageerror = null;
      };
      const fail = (message: string) => {
        finish();
        this.worker.terminate();
        reject(new SigningError(message));
      };
      this.reject = (error) => fail(error.message);
      this.timer = setTimeout(() => fail("Local signing timed out; select the IPA again"), 300_000);
      this.worker.onerror = (event) => {
        event.preventDefault();
        fail("The local signing worker stopped; select the IPA again");
      };
      this.worker.onmessageerror = () => fail("The signing worker returned an unreadable result");
      this.worker.onmessage = (event: MessageEvent<WorkerResponse>) => {
        const response = event.data;
        if (response.type === "progress") {
          onProgress(response.message);
        } else if (response.type === "error") {
          fail(response.message);
        } else {
          finish();
          resolve(response);
        }
      };
      try {
        const transfer =
          request.type === "sign"
            ? [
                request.assets.profile.buffer as ArrayBuffer,
                request.assets.entitlementDocument.buffer as ArrayBuffer,
              ]
            : [];
        this.worker.postMessage(request, transfer);
      } catch {
        fail("Could not start the local signing operation");
      }
    });
  }

  dispose() {
    this.reject?.(new SigningError("Operation cancelled"));
    this.worker.terminate();
  }
}
