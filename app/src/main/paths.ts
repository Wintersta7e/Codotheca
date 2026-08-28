/**
 * Where the app data lives, and where the core binary is.
 */
import * as path from 'node:path';

export interface DataDirInputs {
  /** Electron's `app.getPath('userData')`. */
  userDataPath: string;
  env: NodeJS.ProcessEnv;
}

/**
 * The shell decides, once, and passes the answer to the core in argv. Electron's `userData`
 * and Rust's own directory conventions disagree on Linux; letting each side pick is two
 * databases.
 */
export function resolveDataDir(inputs: DataDirInputs): string {
  const override = inputs.env['CODOTHECA_DATA_DIR'];
  return override !== undefined && override.length > 0 ? override : inputs.userDataPath;
}

export interface CoreBinaryInputs {
  isPackaged: boolean;
  /** Electron's `process.resourcesPath`. */
  resourcesPath: string;
  /** The repository root in development. */
  appRoot: string;
  platform: NodeJS.Platform;
}

export function resolveCoreBinary(inputs: CoreBinaryInputs): string {
  const name = inputs.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core';
  return inputs.isPackaged
    ? path.join(inputs.resourcesPath, 'core', name)
    : path.join(inputs.appRoot, 'core', 'target', 'release', name);
}
