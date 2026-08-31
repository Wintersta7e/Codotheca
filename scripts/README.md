# Scripts

| Script | What it does | How to run |
|---|---|---|
| `check-bundle.mjs` | Post-build guard: the bundle exists where packaging expects it, the renderer's HTML and CSS name no remote origin, the CSP shipped, the three type families were emitted locally, and every packaging glob matches something | `npm run check:bundle` (after `npm run build:app`) |
| `check-stdout-discipline.mjs` | Fails if anything under `core/src` writes to stdout outside `core/src/proto/transport.rs`. stdout carries protocol frames and nothing else | `npm run lint:stdout` |
| `check-style-tokens.mjs` | Criterion 46: the renderer stylesheet declares no colour outside `tokens.css`, no `var(--x)` that is not declared there, and no duration or timing function §11.6 does not carry. Also R35(b): every `@keyframes` name is declared exactly once, and `viewIn`, `panelIn`, `turnIn` only in `styles/base.css`. Prints what it scanned and fails on an empty scan | `npm run lint:style` |
| `win-prepare.mjs` | Makes a WSL-installed `node_modules` usable from Windows too: mirrors every linux native binding to a same-version win32 one, and writes the `.cmd` shims `cmd.exe` needs. **Run after any `npm install` done from WSL** | `npm run prepare:win` |
