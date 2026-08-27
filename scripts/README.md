# Scripts

| Script | What it does | How to run |
|---|---|---|
| `check-bundle.mjs` | Post-build guard: the bundle exists where packaging expects it, the renderer's HTML and CSS name no remote origin, the CSP shipped, the three type families were emitted locally, and every packaging glob matches something | `npm run check:bundle` (after `npm run build:app`) |
| `win-prepare.mjs` | Makes a WSL-installed `node_modules` usable from Windows too: mirrors every linux native binding to a same-version win32 one, and writes the `.cmd` shims `cmd.exe` needs. **Run after any `npm install` done from WSL** | `npm run prepare:win` |
