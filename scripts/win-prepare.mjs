#!/usr/bin/env node
/**
 * Make a WSL-installed `node_modules` usable from Windows as well.
 *
 * Development happens in WSL but the targets are Windows-native, and one `node_modules`
 * cannot serve both out of the box. An `npm install` from either side breaks the other in
 * two ways, and this script repairs both. Run it after any `npm install` done from WSL.
 *
 *  1. **Native bindings.** npm installs the optional platform packages for the *current*
 *     platform only and prunes the other's, so a Windows build dies with
 *     "Cannot find module @rollup/rollup-win32-x64-msvc". Reinstalling from Windows just
 *     inverts the problem. Every linux native package therefore gets a win32 sibling of the
 *     same version placed beside it — the version has to match, because esbuild refuses a
 *     binary whose version differs from its wrapper and reports it as the near-useless
 *     "Error: The service was stopped".
 *
 *  2. **Bin shims.** A WSL install writes bare symlinks into `node_modules/.bin`. `cmd.exe`
 *     cannot execute an extensionless symlink, so every Windows `npm run <script>` fails with
 *     "'vite' is not recognized as an internal or external command" while the same command
 *     works from WSL — which reads as a PATH problem rather than an install artifact.
 *
 * Idempotent: existing win32 packages at the right version and existing shims are left alone.
 */
import { execFileSync } from 'node:child_process';
import {
  cpSync,
  existsSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  readlinkSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');

/** linux package name -> the win32 package that must sit beside it at the same version. */
const MIRRORS = new Map([
  ['@esbuild/linux-x64', '@esbuild/win32-x64'],
  ['@rollup/rollup-linux-x64-gnu', '@rollup/rollup-win32-x64-msvc'],
  ['@rollup/rollup-linux-x64-musl', '@rollup/rollup-win32-x64-msvc'],
]);

function packageVersion(dir) {
  return JSON.parse(
    execFileSync(
      'node',
      ['-p', `JSON.stringify(require(${JSON.stringify(join(dir, 'package.json'))}).version)`],
      { encoding: 'utf8' },
    ),
  );
}

/** Every `node_modules` directory in the tree, at any nesting depth. */
function nodeModulesDirs(from, found = []) {
  for (const entry of readdirSync(from, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const full = join(from, entry.name);
    if (entry.name === 'node_modules') {
      found.push(full);
      nodeModulesDirs(full, found);
    } else if (!entry.name.startsWith('.')) {
      nodeModulesDirs(full, found);
    }
  }
  return found;
}

/** Fetch `name@version` from the registry and unpack it at `destination`. */
function installPackage(name, version, destination) {
  const staging = mkdtempSync(join(tmpdir(), 'win-prepare-'));
  try {
    const tarball = execFileSync(
      'npm',
      ['pack', `${name}@${version}`, '--pack-destination', staging, '--silent'],
      { encoding: 'utf8' },
    ).trim();
    execFileSync('tar', ['-xzf', join(staging, tarball), '-C', staging]);
    mkdirSync(dirname(destination), { recursive: true });
    rmSync(destination, { recursive: true, force: true });
    // cpSync, not renameSync: the staging dir is on / and node_modules is on a mounted
    // volume, and a cross-device rename is EXDEV.
    cpSync(join(staging, 'package'), destination, { recursive: true });
  } finally {
    rmSync(staging, { recursive: true, force: true });
  }
}

let installed = 0;
let alreadyRight = 0;

for (const dir of nodeModulesDirs(root)) {
  for (const [linuxName, winName] of MIRRORS) {
    const linuxDir = join(dir, linuxName);
    if (!existsSync(linuxDir)) continue;
    const version = packageVersion(linuxDir);
    const winDir = join(dir, winName);
    if (existsSync(winDir) && packageVersion(winDir) === version) {
      alreadyRight += 1;
      continue;
    }
    installPackage(winName, version, winDir);
    console.error(`win-prepare: ${winName}@${version} -> ${relative(root, winDir)}`);
    installed += 1;
  }
}

/** npm's own cmd-shim template, with the target path substituted in. */
function cmdShim(targetFromBin) {
  const windowsPath = targetFromBin.replaceAll('/', '\\');
  return [
    '@ECHO off',
    'GOTO start',
    ':find_dp0',
    'SET dp0=%~dp0',
    'EXIT /b',
    ':start',
    'SETLOCAL',
    'CALL :find_dp0',
    '',
    'IF EXIST "%dp0%\\node.exe" (',
    '  SET "_prog=%dp0%\\node.exe"',
    ') ELSE (',
    '  SET "_prog=node"',
    '  SET PATHEXT=%PATHEXT:;.JS;=;%',
    ')',
    '',
    `endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\\${windowsPath}" %*`,
    '',
  ].join('\r\n');
}

let shims = 0;

for (const dir of nodeModulesDirs(root)) {
  const bin = join(dir, '.bin');
  if (!existsSync(bin)) continue;
  for (const entry of readdirSync(bin)) {
    if (entry.endsWith('.cmd') || entry.endsWith('.ps1')) continue;
    const full = join(bin, entry);
    if (!lstatSync(full).isSymbolicLink()) continue;
    if (existsSync(`${full}.cmd`)) continue;
    const target = readlinkSync(full);
    const fromBin = target.startsWith('/') ? relative(bin, resolve(target)) : target;
    writeFileSync(`${full}.cmd`, cmdShim(fromBin), 'utf8');
    shims += 1;
  }
}

console.error(
  `win-prepare: ${String(installed)} win32 package(s) installed, ` +
    `${String(alreadyRight)} already correct, ${String(shims)} .cmd shim(s) written`,
);
