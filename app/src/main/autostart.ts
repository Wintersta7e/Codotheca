// §11.3's `Start with the system`, default off, asked once.
//
// Two implementations because Electron's `app.setLoginItemSettings` is a no-op on Linux, and a
// switch that silently does nothing there is exactly the dead switch §11.3a forbids. Linux
// gets a real XDG autostart file behind the same `AutostartPort` shape.

export const XDG_AUTOSTART_FILE = 'codotheca.desktop';

export interface AutostartPort {
  get(): boolean;
  set(on: boolean): void;
}

export interface AutostartDeps {
  platform: NodeJS.Platform;
  loginItem: { get(): boolean; set(on: boolean): void };
  configHome: string;
  execPath: string;
  fs: {
    write(path: string, text: string): void;
    remove(path: string): void;
    exists(path: string): boolean;
    mkdirp(path: string): void;
  };
}

/** §0: no account and no telemetry, and this file is one of the few that leaves the app. */
export function desktopEntry(execPath: string): string {
  return [
    '[Desktop Entry]',
    'Type=Application',
    'Name=Codotheca',
    `Exec=${execPath} --hidden`,
    'Terminal=false',
    'X-GNOME-Autostart-enabled=true',
    '',
  ].join('\n');
}

export function autostartFor(deps: AutostartDeps): AutostartPort {
  if (deps.platform === 'linux') {
    const dir = `${deps.configHome}/autostart`;
    const path = `${dir}/${XDG_AUTOSTART_FILE}`;
    return {
      // The file's existence is the setting; there is no second place to disagree with it.
      get: () => deps.fs.exists(path),
      set: (on) => {
        if (on) {
          deps.fs.mkdirp(dir);
          deps.fs.write(path, desktopEntry(deps.execPath));
        } else {
          deps.fs.remove(path);
        }
      },
    };
  }
  return { get: () => deps.loginItem.get(), set: (on) => deps.loginItem.set(on) };
}
