/**
 * Electron main process.
 *
 * Owns the window, the tray, the single-instance lock, and the lifecycle of the Rust core.
 * It does NOT open the database: the core is the only reader and writer, and every renderer
 * read crosses the protocol. See `.dev/spec-phase1.md` §2.
 */
import { app, BrowserWindow } from 'electron';
import * as path from 'node:path';
import { PROTOCOL_VERSION } from '../generated/protocol';

// A second instance must focus the first, never start a second core — two cores would be two
// writers against one database. Spec §2.1.
if (!app.requestSingleInstanceLock()) {
  app.quit();
}

let win: BrowserWindow | null = null;

function createWindow(): BrowserWindow {
  const w = new BrowserWindow({
    width: 1440,
    height: 900,
    show: false,
    backgroundColor: '#111417',
    webPreferences: {
      preload: path.join(__dirname, '../preload/index.js'),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
    },
  });
  void w.loadFile(path.join(__dirname, '../../static/index.html'));
  w.once('ready-to-show', () => w.show());
  return w;
}

app.whenReady().then(
  () => {
    process.stderr.write(`codotheca shell, protocol v${PROTOCOL_VERSION}\n`);
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
