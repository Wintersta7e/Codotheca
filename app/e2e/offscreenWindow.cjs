'use strict';

// Main-process preload for every e2e launch (`launchApp.ts` passes it as `-r`); test-only, the
// product never loads it. It keeps a test run off the screens of whoever is using the machine.
const { app, screen } = require('electron');

app.on('browser-window-created', (_event, win) => {
  const displays = screen.getAllDisplays().map((display) => display.bounds);
  const left = Math.min(...displays.map((bounds) => bounds.x));
  const top = Math.min(...displays.map((bounds) => bounds.y));
  win.setPosition(left - win.getBounds().width - 200, top);
  // No taskbar button and never activated: a window the user can reach is one they can minimise
  // or type into. Once minimised on Windows, every CSS animation stayed pending at time 0, even
  // after a restore, and a test read a hero frozen at its first frame (268×8). Playwright
  // emulates page focus, so the page still reads focused.
  win.setSkipTaskbar(true);
  win.show = () => {
    win.showInactive();
  };
  win.focus = () => undefined;
});
