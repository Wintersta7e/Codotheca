/**
 * Electron main process.
 *
 * Owns the window, the tray, the single-instance lock, and the lifecycle of the Rust core.
 * It does NOT open the database: the core is the only reader and writer, and every renderer
 * read crosses the protocol (§1.10, §2.4).
 */
import { app, BrowserWindow, protocol, session } from 'electron';
import * as path from 'node:path';
import { pathToFileURL } from 'node:url';
import { PROTOCOL_VERSION } from '../generated/protocol';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy } from '../shared/csp';
import { EFFECTS_TIER_FLAG } from '../shared/effectsTier';
import { bootstrap, clearPaintFailure } from './bootstrap';
import { readBootFile, writeBootFile } from './bootStore';
import {
  contentSecurityPolicyListener,
  denyPermissionRequest,
  isNavigationAllowed,
} from './security';

// A second instance must focus the first, never start a second core — two cores would be two
// writers against one database. Spec §2.1.
if (!app.requestSingleInstanceLock()) {
  app.quit();
}

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
      additionalArguments: [`${EFFECTS_TIER_FLAG}${boot.tier}`],
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

app.whenReady().then(
  () => {
    const policy =
      rendererUrl === undefined
        ? CONTENT_SECURITY_POLICY
        : developmentContentSecurityPolicy(new URL(rendererUrl).origin);
    session.defaultSession.webRequest.onHeadersReceived(contentSecurityPolicyListener(policy));
    session.defaultSession.setPermissionRequestHandler(denyPermissionRequest);

    process.stderr.write(`codotheca shell, protocol v${String(PROTOCOL_VERSION)}\n`);
    win = createWindow();
    app.on('second-instance', () => {
      if (win) {
        if (win.isMinimized()) win.restore();
        win.focus();
      }
    });
  },
  (e: unknown) => {
    process.stderr.write(`failed to start: ${String(e)}\n`);
    app.quit();
  },
);

// Resident by design once the tray lands (spec §0); for now, quit with the last window.
app.on('window-all-closed', () => app.quit());
