import init, { WasmSigner } from "@jveko/zsign-wasm";
import wasmUrl from "@jveko/zsign-wasm/zsign_wasm_bg.wasm?url";
import forge from "node-forge";

import { readArchive, writeArchive, type ArchiveFile } from "./archive";
import { inspectMachO } from "./macho";
import {
  SigningError,
  signingLimits,
  type Inspection,
  type WorkerRequest,
  type WorkerResponse,
} from "./types";

const scope = globalThis as unknown as {
  onmessage: ((event: MessageEvent<WorkerRequest>) => void) | null;
  postMessage: (message: WorkerResponse, transfer?: Transferable[]) => void;
};
let files: ArchiveFile[] | undefined;
let root = "";
let inspection: Inspection | undefined;
let busy = false;
const progress = (message: string) => scope.postMessage({ type: "progress", message });

async function inspect(file: File) {
  progress("Inspecting local archive…");
  files = await readArchive(file);
  await init({ module_or_path: wasmUrl });
  const roots = files.filter((entry) => /^Payload\/[^/]+\.app\/Info\.plist$/.test(entry.name));
  if (roots.length !== 1) {
    throw new SigningError("The IPA must contain exactly one primary app");
  }
  const info = roots[0];
  root = info.name.slice(0, -"Info.plist".length);
  for (const entry of files) {
    if (
      !entry.name.startsWith(root) ||
      /\.(app|appex|framework|xpc|bundle)\//i.test(entry.name.slice(root.length))
    ) {
      throw new SigningError(
        "This first version supports one app only, without nested bundles or frameworks",
      );
    }
  }
  if (info.data.length > signingLimits.plistBytes) {
    throw new SigningError("Info.plist is too large");
  }
  const metadata: unknown = WasmSigner.parse_info_plist(info.data);
  if (
    !metadata ||
    typeof metadata !== "object" ||
    !("bundle_id" in metadata) ||
    !("executable" in metadata) ||
    typeof metadata.bundle_id !== "string" ||
    typeof metadata.executable !== "string" ||
    !/^[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+$/.test(metadata.bundle_id) ||
    metadata.bundle_id.length > 255 ||
    !metadata.executable ||
    /[/\\\0]/.test(metadata.executable) ||
    metadata.executable === "." ||
    metadata.executable === ".."
  ) {
    throw new SigningError("The app has an invalid bundle identifier or executable");
  }
  const entitlements: string[] = [];
  let found = false;
  for (const entry of files) {
    const values = inspectMachO(entry.data);
    if (entry.name === root + metadata.executable) {
      if (!values) {
        throw new SigningError("The app executable is not a supported Mach-O");
      }
      found = true;
      entry.executable = true;
      entitlements.push(...values);
    } else if (values) {
      throw new SigningError("Additional executables and dynamic libraries are not supported yet");
    }
  }
  if (!found) {
    throw new SigningError("The app executable is missing");
  }
  inspection = { bundleId: metadata.bundle_id, executable: metadata.executable, entitlements };
  scope.postMessage({ type: "inspected", inspection });
}

async function sign(assets: Extract<WorkerRequest, { type: "sign" }>["assets"]) {
  if (!files || !inspection) {
    throw new SigningError("Inspect an IPA first");
  }
  progress("Signing app locally…");
  const certificate = forge.pki.certificateFromAsn1(
    forge.asn1.fromDer(forge.util.decode64(assets.certificate)),
  );
  const container = forge.pkcs12.toPkcs12Asn1(
    forge.pki.privateKeyFromPem(assets.privateKeyPem),
    [certificate],
    "",
    { algorithm: "3des" },
  );
  const p12 = Uint8Array.from(forge.asn1.toDer(container).getBytes(), (value) =>
    value.charCodeAt(0),
  );
  let signer: WasmSigner | undefined;
  try {
    signer = new WasmSigner(p12, "", assets.entitlementDocument);
    if (!signer.entitlements()) {
      throw new SigningError("The signer could not load the validated entitlements");
    }
    files = files.filter(
      (file) =>
        !file.name.startsWith(root + "_CodeSignature/") &&
        file.name !== root + "embedded.mobileprovision",
    );
    files.push({
      name: root + "embedded.mobileprovision",
      data: assets.profile,
      executable: false,
    });
    signer.set_main_executable(inspection.executable);
    for (const file of files) {
      signer.hash_file(file.name.slice(root.length), file.data);
    }
    const resources = signer.build_code_resources();
    const executable = files.find((file) => file.name === root + inspection?.executable);
    const info = files.find((file) => file.name === root + "Info.plist");
    if (!executable || !info) {
      throw new SigningError("The inspected app is unavailable");
    }
    executable.data = signer.sign_macho_fat(
      executable.data,
      inspection.bundleId,
      info.data,
      resources,
    );
    files.push({ name: root + "_CodeSignature/CodeResources", data: resources, executable: false });
    progress("Packaging signed result…");
    const output = await writeArchive(files);
    const bytes = new Uint8Array(output).buffer;
    scope.postMessage({ type: "signed", bytes }, [bytes]);
  } finally {
    p12.fill(0);
    signer?.free();
    files = undefined;
    inspection = undefined;
  }
}

scope.onmessage = (event) => {
  if (busy) {
    return;
  }
  busy = true;
  const request = event.data;
  const operation = request.type === "inspect" ? inspect(request.file) : sign(request.assets);
  void operation
    .catch((error: unknown) => {
      files = undefined;
      inspection = undefined;
      // Never forward signer/crypto errors: they may include foreign input.
      scope.postMessage({
        type: "error",
        message:
          error instanceof SigningError
            ? error.message
            : "The local signer could not process this IPA or signing identity",
      });
    })
    .finally(() => {
      busy = false;
    });
};
