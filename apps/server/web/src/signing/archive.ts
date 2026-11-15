import {
  BlobReader,
  Uint8ArrayReader,
  Uint8ArrayWriter,
  ZipReader,
  ZipWriter,
} from "@zip.js/zip.js";

import { SigningError, signingLimits } from "./types";

export interface ArchiveFile {
  name: string;
  data: Uint8Array;
  executable: boolean;
}

export async function readArchive(file: File): Promise<ArchiveFile[]> {
  if (!file.size || file.size > signingLimits.inputBytes) {
    throw new SigningError("Choose an IPA smaller than 100 MiB");
  }
  const reader = new ZipReader(new BlobReader(file), { useWebWorkers: false });
  const files: ArchiveFile[] = [];
  const names = new Set<string>();
  let total = 0;
  try {
    for await (const entry of reader.getEntriesGenerator()) {
      const name = entry.filename;
      if (
        names.size >= signingLimits.entries ||
        names.has(name) ||
        name.length > 1024 ||
        name.startsWith("/") ||
        name.includes("\\") ||
        name.includes("\0") ||
        name
          .split("/")
          .some(
            (part, index, parts) =>
              part === "." || part === ".." || (!part && index !== parts.length - 1),
          )
      ) {
        throw new SigningError("Unsafe, duplicate, or excessive archive entries");
      }
      names.add(name);
      const mode = entry.externalFileAttributes >>> 16;
      const kind = mode & 0xf000;
      if (entry.encrypted || (kind !== 0 && kind !== 0x8000 && kind !== 0x4000)) {
        throw new SigningError("Encrypted archives, symlinks, and special files are unsupported");
      }
      if (entry.directory) {
        continue;
      }
      if (
        entry.uncompressedSize > signingLimits.entryBytes ||
        total + entry.uncompressedSize > signingLimits.expandedBytes
      ) {
        throw new SigningError("IPA exceeds the 256 MiB expanded archive limit");
      }
      const chunks: Uint8Array[] = [];
      let size = 0;
      await entry.getData(
        new WritableStream<Uint8Array>({
          write(chunk) {
            size += chunk.length;
            total += chunk.length;
            if (
              size > entry.uncompressedSize ||
              size > signingLimits.entryBytes ||
              total > signingLimits.expandedBytes
            ) {
              throw new SigningError("IPA expanded beyond its declared size or memory limit");
            }
            chunks.push(chunk);
          },
        }),
        { checkSignature: true, useWebWorkers: false },
      );
      if (size !== entry.uncompressedSize) {
        throw new SigningError("Invalid archive entry size");
      }
      const data = new Uint8Array(size);
      let offset = 0;
      for (const chunk of chunks) {
        data.set(chunk, offset);
        offset += chunk.length;
      }
      files.push({ name, data, executable: (mode & 0o111) !== 0 });
    }
  } finally {
    await reader.close();
  }
  return files;
}

export async function writeArchive(files: ArchiveFile[]): Promise<Uint8Array> {
  const writer = new ZipWriter(new Uint8ArrayWriter(), { useWebWorkers: false });
  for (const file of files) {
    await writer.add(file.name, new Uint8ArrayReader(file.data), {
      level: 0,
      zip64: false,
      lastModDate: new Date(1980, 0, 1),
      externalFileAttributes: ((file.executable ? 0o100755 : 0o100644) << 16) >>> 0,
      versionMadeBy: 3 << 8,
    });
  }
  return writer.close();
}
