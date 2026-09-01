/**
 * Electron main process.
 *
 * Owns the window, the tray, the single-instance lock, and the lifecycle of the Rust core.
 * It does NOT open the database: the core is the only reader and writer, and every renderer
 * read crosses the protocol (§1.10, §2.4).
 */
import { BrowserWindow, app, dialog, ipcMain, protocol, session } from 'electron';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import { type CommandName, PROTOCOL_VERSION, type Topic } from '../generated/protocol';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy } from '../shared/csp';
import {
  EFFECTS_TIER_FLAG,
  EFFECTS_TIER_SOURCE_FLAG,
  PAINT_FAIL_FORCED_AT_FLAG,
} from '../shared/effectsTier';
import { registerArtProtocol, readRenditionFromDisk } from './art/artProtocol';
import { bootstrap, clearPaintFailure } from './bootstrap';
import { readBootFile, writeBootFile } from './bootStore';
import { type BridgeRequest, registerBridge } from './core/bridge';
import { registerRelocateDialog } from './dialogs/relocate';
import { CoreClient } from './core/client';
import { FORCE_OFFERED_AFTER_MS, LOCK_WAIT_POLL_MS, waitForCoreLock } from './core/instanceLock';
import { openRollingLog } from './core/log';
import { spawnCoreChild } from './core/spawn';
import { CoreSupervisor } from './core/supervisor';
import { resolveCoreBinary, resolveDataDir } from './paths';
import { registerFocusRelease } from './session/focus';
import { launchJoinSteps } from './startup/launchSteps';
import {
  contentSecurityPolicyListener,
  denyPermissionRequest,
  isNavigationAllowed,
} from './security';
import { runStartup } from './startup';

const TOPICS: Topic[] = ['scan', 'projects', 'session', 'core'];

/**
 * Deliberately empty: no command has a handler yet, so nothing is callable from the renderer
 * and the bridge refuses every name. A list naming commands with no handler behind them is the
 * dead-control defect §11 exists to correct.
 */
const KNOWN_COMMANDS: readonly CommandName[] = [];

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
    dataDir,
    log,
    spawn: spawnCoreChild,
    now: () => Date.now(),
    schedule: (fn, ms) => {
      setTimeout(fn, ms).unref();
    },
  });

  const client = new CoreClient(supervisor);
  const request = client.request.bind(client) as unknown as BridgeRequest;

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
    onStatus: (fn) => {
      supervisor.onStatus(fn);
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
    // §11.2's launch lane. `verifyTargetsStep` is plan 17's and is not written yet, so the
    // lane is one step short rather than carrying a second declaration of it.
    joinSteps: [
      ...launchJoinSteps({
        request: (name, args) => client.request(name as never, args as never),
        subscribe: (topic, handler) => client.subscribe(topic, handler),
        onRecovered: (sessions) => {
          if (sessions.length > 0) {
            log.write('info', 'shell', `recovered ${String(sessions.length)} orphaned session(s)`);
          }
        },
        onVerified: () => undefined,
      }),
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
  app.on('before-quit', () => {
    supervisor.stop();
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
