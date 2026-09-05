/**
 * Electron main process.
 *
 * Owns the window, the tray, the single-instance lock, and the lifecycle of the Rust core.
 * It does NOT open the database: the core is the only reader and writer, and every renderer
 * read crosses the protocol (§1.10, §2.4).
 */
import {
  BrowserWindow,
  app,
  dialog,
  globalShortcut,
  ipcMain,
  protocol,
  session,
  shell,
} from 'electron';
import { existsSync, readFileSync, statSync } from 'node:fs';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import {
  type CommandName,
  PROTOCOL_VERSION,
  type RootAdd,
  type RootSuggestion,
  type Topic,
} from '../generated/protocol';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy } from '../shared/csp';
import {
  EFFECTS_TIER_FLAG,
  EFFECTS_TIER_SOURCE_FLAG,
  PAINT_FAIL_FORCED_AT_FLAG,
} from '../shared/effectsTier';
import { logPathArgument } from '../shared/windowArgs';
import { startShortcutService, withShortcutRebind } from './paletteShortcut';
import { registerShellServices } from './shellServices';
import { readStartupFailure } from './startupFailure';
import { registerArtProtocol, readRenditionFromDisk } from './art/artProtocol';
import { bootstrap, clearPaintFailure } from './bootstrap';
import { readBootFile, writeBootFile } from './bootStore';
import { type BridgeRequest, registerBridge } from './core/bridge';
import { registerRelocateDialog } from './dialogs/relocate';
import { registerRootPicker, registerSuggestionCommit, SuggestionCache } from './rootPicker';
import { CoreClient } from './core/client';
import { FORCE_OFFERED_AFTER_MS, LOCK_WAIT_POLL_MS, waitForCoreLock } from './core/instanceLock';
import { openRollingLog } from './core/log';
import { spawnCoreChild } from './core/spawn';
import { CoreSupervisor } from './core/supervisor';
import { resolveCoreBinary, resolveDataDir, resolveWorkerBinary, WORKER_ARCHES } from './paths';
import { installQuitGate } from './quitGate';
import { formatArtifactStamp, readArtifactStamp } from './update/artifact';
import { registerFocusRelease } from './session/focus';
import { logLevelStep, residentShortcutStep, staleTargets, verifyTargetsStep } from './joinSteps';
import { launchJoinSteps } from './startup/launchSteps';
import {
  contentSecurityPolicyListener,
  denyPermissionRequest,
  isNavigationAllowed,
} from './security';
import { runStartup } from './startup';

const TOPICS: Topic[] = ['scan', 'projects', 'session', 'core', 'accounts'];

/**
 * The commands the core answers, and therefore the only names the bridge will accept.
 *
 * A name belongs here exactly when the core assembly route maps it to a handler. The two
 * supervisor-channel commands are deliberately absent: the shell sends those itself, and the
 * renderer never names either one.
 */
export const KNOWN_COMMANDS: readonly CommandName[] = [
  // firstrun::dispatch
  'roots.suggest',
  'roots.list',
  'roots.add',
  'roots.remove',
  'roots.setEnabled',
  'roots.setDescend',
  'stats.reveal',
  'identity.list',
  'identity.confirm',
  // art::dispatch_art_command
  'art.url',
  'art.rerender',
  // commands::launch::dispatch_launch_command
  'projects.launch',
  'session.stop',
  'session.focus',
  // commands::targets::dispatch_targets_command
  'targets.list',
  'targets.setDefault',
  'targets.upsert',
  'targets.verify',
  // surfaces::dispatch_surface_command
  'problems.list',
  'settings.get',
  'settings.set',
  'locations.setTrusted',
  'projects.requeue',
  'diag.bundle',
  // scan::dispatch_scan_command
  'scan.start',
  'scan.cancel',
  'scan.status',
  // identity::commands::dispatch_identity_command
  'projects.merge',
  'projects.unmergeHint',
  // projects::dispatch_projects_command
  'projects.list',
  'projects.peek',
  'projects.setFlags',
  // detail::dispatch_detail_command
  'projects.get',
  'projects.setNote',
  'locations.relocate',
  // view::dispatch_view_command
  'view.get',
  'view.set',
  'collections.list',
  'collections.upsert',
  'collections.remove',
  // accounts::dispatch_accounts_command. The other five accounts.* names are still unowned in
  // the core, and `app/test/knownCommands.test.ts` refuses a bridge that offers one of those.
  'accounts.list',
  'accounts.orgs',
  'accounts.setOrgEnabled',
  'accounts.connect',
  'accounts.cancelConnect',
];

// A second instance must focus the first, never start a second core — two cores would be two
// writers against one database. Asked here rather than inside the startup sequence because
// Electron wants it before the app does any real work. Spec §2.1.
const hasInstanceLock = app.requestSingleInstanceLock();

// electron-vite sets this while `electron-vite dev` is running, and never in a packaged app.
const rendererUrl = process.env['ELECTRON_RENDERER_URL'];

const boot = bootstrap({
  argv: process.argv,
  env: process.env,
  userDataDir: app.getPath('userData'),
  registerSchemesAsPrivileged: (schemes) => protocol.registerSchemesAsPrivileged(schemes),
  disableHardwareAcceleration: () => app.disableHardwareAcceleration(),
  readBoot: readBootFile,
  writeBoot: writeBootFile,
});

let win: BrowserWindow | null = null;
// Set before the window is created, in `main`. The renderer needs it on its first frame —
// §11.2a's failure windows name the log — and a round trip is exactly what §11.2 forbids there.
let logPath = '';

function entryUrl(): string {
  // pathToFileURL, not string concatenation: on Windows a drive-letter path concatenated
  // onto `file://` parses the drive letter as the URL host and denies every navigation.
  return rendererUrl ?? pathToFileURL(path.join(__dirname, '../renderer/index.html')).href;
}

function createWindow(): BrowserWindow {
  const w = new BrowserWindow({
    width: 1440,
    height: 900,
    show: false,
    // --app-bg, so a cold start never flashes white behind the shelf.
    backgroundColor: '#07090b',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      webviewTag: false,
      // §11.2a: the tier and the account of where it came from both reach the document with
      // no round trip, because the core joins after first paint.
      additionalArguments: [
        `${EFFECTS_TIER_FLAG}${boot.tier}`,
        `${EFFECTS_TIER_SOURCE_FLAG}${boot.source}`,
        ...(boot.stored.paintFailForcedAt === null
          ? []
          : [`${PAINT_FAIL_FORCED_AT_FLAG}${String(boot.stored.paintFailForcedAt)}`]),
        logPathArgument(logPath),
      ],
    },
  });

  const entry = entryUrl();
  w.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  w.webContents.on('will-navigate', (event, url) => {
    if (!isNavigationAllowed(entry, url)) {
      event.preventDefault();
    }
  });

  if (rendererUrl === undefined) {
    void w.loadFile(path.join(__dirname, '../renderer/index.html'));
  } else {
    void w.loadURL(rendererUrl);
  }
  w.once('ready-to-show', () => {
    // Electron's first-composited-frame signal. If the GPU wedged, it never fires and the
    // counter bootstrap incremented survives into the next launch — which is the mechanism.
    clearPaintFailure({
      userDataDir: app.getPath('userData'),
      readBoot: readBootFile,
      writeBoot: writeBootFile,
    });
    w.show();
  });
  return w;
}

/**
 * §13's worker on this machine, or `null`.
 *
 * Windows only: `\\wsl$\` paths exist nowhere else, so a Linux build has no bridge to cross
 * and passing a path would only invite the core to read a file it will never use. The arch is
 * this process's, because the worker runs inside a distro on this same machine.
 */
function stagedWorkerPath(): string | null {
  if (process.platform !== 'win32') {
    return null;
  }
  const arch = WORKER_ARCHES.find((candidate) => candidate === process.arch);
  if (arch === undefined) {
    return null;
  }
  const staged = resolveWorkerBinary({
    isPackaged: app.isPackaged,
    resourcesPath: process.resourcesPath,
    appRoot: path.join(app.getAppPath(), '..'),
    arch,
  });
  return existsSync(staged) ? staged : null;
}

async function main(): Promise<void> {
  await app.whenReady();

  const policy =
    rendererUrl === undefined
      ? CONTENT_SECURITY_POLICY
      : developmentContentSecurityPolicy(new URL(rendererUrl).origin);
  session.defaultSession.webRequest.onHeadersReceived(contentSecurityPolicyListener(policy));
  session.defaultSession.setPermissionRequestHandler(denyPermissionRequest);

  process.stderr.write(`codotheca shell, protocol v${String(PROTOCOL_VERSION)}\n`);

  const dataDir = resolveDataDir({ userDataPath: app.getPath('userData'), env: process.env });
  const log = openRollingLog({
    dir: path.join(dataDir, 'logs'),
    maxBytes: 4 * 1024 * 1024,
    keep: 3,
    level: 'info',
  });
  logPath = log.path;

  // §11.4: a diagnostics bundle has to name the build it came from, and neither half of that
  // name is a compile-time constant — the version is written into the packaged manifest after
  // the bundler has run, and which of the five artifacts is executing is only observable at
  // run time. It goes to the rolling log, which is what the bundle collects.
  log.write(
    'info',
    'shell',
    `artifact: ${formatArtifactStamp(
      readArtifactStamp({
        appPath: app.getAppPath(),
        readTextFile: (p) => readFileSync(p, 'utf8'),
        artifact: { isPackaged: app.isPackaged, platform: process.platform, env: process.env },
      }),
    )}`,
  );

  // After `app.ready`, before the window: the renderer cannot load `file:`, so a card that
  // paints before this is bound would 404 and fall back to the nameplate for its first frame.
  registerArtProtocol(protocol, { dataDir, readRendition: readRenditionFromDisk, log });

  const supervisor = new CoreSupervisor({
    binaryPath: resolveCoreBinary({
      isPackaged: app.isPackaged,
      resourcesPath: process.resourcesPath,
      appRoot: path.join(app.getAppPath(), '..'),
      platform: process.platform,
    }),
    // §13. Only Windows has a bridge to cross, and only a build that staged the ELF has
    // anything to run inside a distro. Absent is passed as absent rather than as a path that
    // does not resolve, so the core reports "no worker" instead of an unreadable file.
    workerPath: stagedWorkerPath(),
    dataDir,
    log,
    spawn: spawnCoreChild,
    now: () => Date.now(),
    schedule: (fn, ms) => {
      setTimeout(fn, ms).unref();
    },
  });

  const client = new CoreClient(supervisor);
  const coreRequest = client.request.bind(client) as unknown as BridgeRequest;

  // §8.6 and R32. The window does not exist yet — the UI lane paints below — so every send goes
  // through `win?`, and the binding republishes its state on each transition after that.
  const shortcut = startShortcutService({
    host: {
      register: (chord, cb) => globalShortcut.register(chord, cb),
      unregister: (chord) => {
        globalShortcut.unregister(chord);
      },
      isRegistered: (chord) => globalShortcut.isRegistered(chord),
    },
    showWindow: () => {
      if (win === null) return;
      if (win.isMinimized()) win.restore();
      win.show();
      win.focus();
    },
    send: (channel, payload) => {
      win?.webContents.send(channel, payload);
    },
  });

  // The drawer rebinds by writing `settings.set`, which otherwise reaches the core and nothing
  // else — the chord would be stored and never registered.
  const request = withShortcutRebind(coreRequest, shortcut.apply);

  registerBridge({
    request,
    subscribe: (topic, onEvent) =>
      client.subscribe(topic, { onEvent, onSnapshot: () => undefined }),
    topics: TOPICS,
    schedule: (fn, ms) => {
      const t = setTimeout(fn, ms);
      return (): void => {
        clearTimeout(t);
      };
    },
    // §11.2a's report is re-read here rather than carried on the window's argv: the core writes
    // it *after* the window exists, so an argv copy is null the first time a fault happens and
    // stale after a repair. The moment the lane is declared failed is when it is true.
    onStatus: (fn) => {
      supervisor.onStatus((status) => {
        fn(
          status.kind === 'failed'
            ? { ...status, startupFailure: readStartupFailure(dataDir) }
            : status,
        );
      });
    },
    handle: (channel, fn) => {
      ipcMain.handle(channel, (_event, payload: unknown) => fn(payload));
    },
    sendToRenderer: (channel, payload) => {
      win?.webContents.send(channel, payload);
    },
    knownCommands: KNOWN_COMMANDS,
  });

  // §2.4: `locations.relocate` is privileged, so the bridge above refuses it by design. It
  // travels its own channel, where the path is whatever this process's dialog returns and never
  // anything the renderer typed.
  registerRelocateDialog({
    handle: (channel, fn) => {
      ipcMain.handle(channel, (_event, payload: unknown) => fn(payload));
    },
    showFolderDialog: async () => {
      const result = await dialog.showOpenDialog({
        properties: ['openDirectory'],
        title: 'Where is this copy now?',
        buttonLabel: 'Relocate',
      });
      return result.canceled || result.filePaths[0] === undefined ? null : result.filePaths[0];
    },
    request,
  });

  // §2.4 again, for the other privileged path: `roots.add` carries `pathBytes`, so the folder
  // comes from this process's dialog and never from the renderer. R11 makes this the only
  // registration on the channel.
  registerRootPicker({
    handle: (channel, fn) => {
      ipcMain.handle(channel, (_event, payload: unknown) => fn(payload));
    },
    showOpenDialog: () =>
      dialog.showOpenDialog({
        properties: ['openDirectory'],
        title: 'Choose a folder to look in',
        buttonLabel: 'Look here',
      }),
    addRoot: (args) => request('roots.add', args) as Promise<RootAdd>,
  });

  // GAP-16b-1: the other half of §10.1b. Ticking only *suggested* roots and pressing `DIG IN`
  // used to add nothing at all, because the display string had nowhere to resolve against.
  registerSuggestionCommit({
    handle: (channel, fn) => {
      ipcMain.handle(channel, (_event, payload: unknown) => fn(payload));
    },
    suggestRoots: () => request('roots.suggest', {}) as Promise<RootSuggestion[]>,
    addRoot: (args) => request('roots.add', args) as Promise<RootAdd>,
    cache: new SuggestionCache(),
  });

  // §11.3 and §11.5: reveal, the index location, the paint-failure reset and the executable
  // dialog. The module landed with plan 17 and was called from nowhere, so every one of those
  // drawer rows invoked a channel with no handler — a dead switch that also rejects.
  registerShellServices({
    handle: (channel, fn) => {
      ipcMain.handle(channel, (_event, payload: unknown) => fn(null, payload));
    },
    // §2.4: the bytes the core stores are the *path* to the executable, chosen in a dialog this
    // process owns. `exec_bytes` is the column; nothing reads the file.
    openExecutable: async () => {
      const result = await dialog.showOpenDialog({
        properties: ['openFile'],
        title: 'Choose an application',
        buttonLabel: 'Use this',
      });
      const chosen = result.canceled ? undefined : result.filePaths[0];
      return chosen === undefined ? null : Buffer.from(chosen);
    },
    revealItem: (target) => {
      shell.showItemInFolder(target);
    },
    dataDir,
    logPath: log.path,
    statSync: (target) => statSync(target),
    request: (name, args) => request(name as CommandName, args),
    readBoot: readBootFile,
    writeBoot: writeBootFile,
  });

  const lockWait = new AbortController();
  const outcome = await runStartup({
    acquireInstanceLock: () => hasInstanceLock,
    focusExistingWindow: () => {
      app.quit();
    },
    dataDir,
    supervisor,
    log,
    now: () => Date.now(),
    paintUiLane: async () => {
      const w = createWindow();
      win = w;
      // §9: the shell releases a focus claim and never makes one. Destroy matters most — the
      // renderer's heartbeat dies with the window, so without it the core would believe the
      // last claim for a further FOCUS_STALE_SECS.
      registerFocusRelease({
        release: (args) => client.request('session.focus', args as never),
        onBlur: (cb) => w.on('blur', cb),
        onHide: (cb) => w.on('hide', cb),
        onDestroyed: (cb) => w.once('closed', cb),
        onError: (detail) => {
          log.write('warn', 'shell', `focus release failed: ${detail}`);
        },
      });
      await new Promise<void>((resolve) => {
        w.once('ready-to-show', () => {
          resolve();
        });
      });
    },
    awaitFreeLock: () =>
      waitForCoreLock(dataDir, {
        pollMs: LOCK_WAIT_POLL_MS,
        onWait: (elapsed) => {
          if (elapsed >= FORCE_OFFERED_AFTER_MS) {
            log.write(
              'warn',
              'shell',
              `another core is still shutting down · ${String(elapsed)}ms`,
            );
          }
        },
        signal: lockWait.signal,
        now: () => Date.now(),
        sleep: (ms) => new Promise<void>((r) => setTimeout(r, ms)),
      }),
    // §11.2's launch lane: recovery first, because a session orphaned by a crash must be
    // closed before anything reads playtime, then the target sweep, which blocks nothing.
    joinSteps: [
      ...launchJoinSteps(
        {
          request: (name, args) => client.request(name as never, args as never),
          subscribe: (topic, handler) => client.subscribe(topic, handler),
          onRecovered: (sessions) => {
            if (sessions.length > 0) {
              log.write(
                'info',
                'shell',
                `recovered ${String(sessions.length)} orphaned session(s)`,
              );
            }
          },
          onVerified: () => undefined,
        },
        verifyTargetsStep(
          (name, args) => client.request(name as never, args as never),
          (rows) => {
            const stale = staleTargets(rows);
            if (stale.length > 0) {
              log.write('warn', 'shell', `${String(stale.length)} launch target(s) missing`);
            }
          },
        ),
      ),
      logLevelStep((name, args) => client.request(name as never, args as never), {
        setLevel: (level) => {
          log.setLevel(level);
        },
      }),
      residentShortcutStep(
        (name, args) => client.request(name as never, args as never),
        shortcut.apply,
      ),
    ],
  });

  log.write('info', 'shell', `startup: ${outcome.kind}`);
  if (outcome.kind === 'running') {
    log.write('info', 'shell', `interactive after ${String(outcome.interactiveAfterMs)} ms`);
    const joined = await outcome.joined;
    log.write('info', 'shell', `core lane: ${joined.kind}`);
  }

  app.on('second-instance', () => {
    if (win === null) return;
    if (win.isMinimized()) win.restore();
    win.focus();
  });
  // The core must be fully gone, not merely told to go: its orderly-close path writes the
  // session's close reason, and a kill mid-write loses it. `stop()` starts a shutdown;
  // `stopAndWait()` completes one.
  //
  // The chord is released first and unconditionally. It is a process-wide OS binding, it costs
  // nothing to drop, and a core shutdown that stalls must not leave it held — a stale global
  // accelerator outlives the window it was meant to raise.
  installQuitGate({
    app,
    steps: [
      {
        name: 'shortcut-release',
        run: () => {
          shortcut.dispose();
          return Promise.resolve('released');
        },
      },
      { name: 'core-shutdown', run: () => supervisor.stopAndWait() },
    ],
    log,
  });
}

// Resident by design once the tray lands (spec §0); for now, quit with the last window.
app.on('window-all-closed', () => {
  app.quit();
});

void main().catch((e: unknown) => {
  process.stderr.write(`failed to start: ${String(e)}\n`);
  app.quit();
});
