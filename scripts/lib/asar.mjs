/**
 * Reads root-level entries out of an Electron `app.asar` without a dependency.
 *
 * An asar starts with an 8-byte pickle: uint32 `4`, then the complete inner-pickle size. The
 * inner pickle is uint32 payload size, uint32 JSON byte length, JSON, then four-byte padding.
 * Packed entry offsets are decimal strings relative to the byte immediately after those two
 * pickles.
 */
import { closeSync, openSync, readSync, statSync } from 'node:fs';

/** The two files every pack must carry, non-empty, at the root of its `app.asar`. */
export const PACKED_LICENCE_FILES = ['LICENSE', 'THIRD-PARTY-NOTICES'];

function readExactly(descriptor, size, position) {
  const output = Buffer.alloc(size);
  let read = 0;
  while (read < size) {
    const count = readSync(descriptor, output, read, size - read, position + read);
    if (count === 0) break;
    read += count;
  }
  if (read !== size) {
    throw new Error(`unexpected end of file at ${String(position + read)}`);
  }
  return output;
}

function readHeader(descriptor, archiveSize) {
  if (archiveSize < 16) throw new Error('archive is shorter than its two pickle headers');
  const prefix = readExactly(descriptor, 8, 0);
  if (prefix.readUInt32LE(0) !== 4) {
    throw new Error('outer pickle payload size is not 4');
  }

  const headerSize = prefix.readUInt32LE(4);
  if (headerSize < 8) throw new Error('inner pickle is shorter than 8 bytes');
  if (headerSize > 64 * 1024 * 1024) throw new Error('inner pickle exceeds 64 MiB');
  if (8 + headerSize > archiveSize) throw new Error('inner pickle extends past the archive');

  const header = readExactly(descriptor, headerSize, 8);
  const payloadSize = header.readUInt32LE(0);
  if (payloadSize !== headerSize - 4) {
    throw new Error('inner pickle payload size does not match its header');
  }
  const jsonSize = header.readUInt32LE(4);
  if (jsonSize > payloadSize - 4) throw new Error('header JSON extends past the inner pickle');
  const paddedPayloadSize = Math.ceil((4 + jsonSize) / 4) * 4;
  if (paddedPayloadSize !== payloadSize) {
    throw new Error('inner pickle padding is inconsistent');
  }

  try {
    return { tree: JSON.parse(header.subarray(8, 8 + jsonSize).toString('utf8')), headerSize };
  } catch (error) {
    throw new Error(`header JSON is invalid: ${String(error)}`);
  }
}

/** A root-level entry's header record, or a thrown reason it is not a packed regular file. */
function packedEntry(tree, name) {
  const entry = tree?.files?.[name];
  if (entry === undefined || entry === null || typeof entry !== 'object') {
    throw new Error(`asar header has no root ${name} entry`);
  }
  if (entry.unpacked === true) throw new Error(`root ${name} is unexpectedly unpacked`);
  if (Object.hasOwn(entry, 'link')) throw new Error(`root ${name} is unexpectedly a link`);
  if (typeof entry.offset !== 'string' || !/^(?:0|[1-9]\d*)$/u.test(entry.offset)) {
    throw new Error(`root ${name} has an invalid packed offset`);
  }
  if (!Number.isSafeInteger(entry.size) || entry.size < 0) {
    throw new Error(`root ${name} has an invalid size`);
  }
  return entry;
}

/** The bytes of one root-level file in the archive. Throws naming what is wrong. */
export function readAsarFile(archivePath, name) {
  const archiveSize = statSync(archivePath).size;
  const descriptor = openSync(archivePath, 'r');
  try {
    const { tree, headerSize } = readHeader(descriptor, archiveSize);
    const entry = packedEntry(tree, name);
    const filePositionBig = 8n + BigInt(headerSize) + BigInt(entry.offset);
    const fileEndBig = filePositionBig + BigInt(entry.size);
    if (fileEndBig > BigInt(archiveSize)) {
      throw new Error(`root ${name} extends past the archive`);
    }
    if (filePositionBig > BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new Error(`root ${name} offset exceeds the safe integer range`);
    }
    return readExactly(descriptor, entry.size, Number(filePositionBig));
  } finally {
    closeSync(descriptor);
  }
}

/**
 * Why the archive does not carry the licence files, one message each; empty when it does.
 * A present but empty file is a failure: a zero-byte notice satisfies a glob and informs nobody.
 */
export function packedLicenceFailures(archivePath) {
  const failures = [];
  for (const name of PACKED_LICENCE_FILES) {
    try {
      if (readAsarFile(archivePath, name).length === 0) {
        failures.push(`${name} is empty in ${archivePath}`);
      }
    } catch (error) {
      failures.push(`${name} is not packed in ${archivePath}: ${error.message}`);
    }
  }
  return failures;
}
