# The WSL worker

Codotheca ships a second binary, `codotheca-worker`: a Linux ELF that runs inside a WSL distro
and speaks the core's frame protocol over `wsl.exe` stdio. It exists because walking the
`\\wsl$` bridge from Windows puts every `stat` through a virtual machine, and because starting
`wsl.exe` once per repository costs more than the work it does.

## What runs where

| Piece | Runs on | Tested |
|---|---|---|
| Path translation, mount classification, the frame vocabulary, the install plan | either | `cargo test` |
| The worker itself — serve loop, in-distro walk, git | inside the distro | `cargo test`, over a loopback socket running the real serve loop |
| `wsl.exe` enumeration and launch; installing the ELF into a distro | Windows with WSL | `--ignored` only |

The loopback tests need a usable `127.0.0.1`. In a sandbox that denies socket creation they fail
at launch rather than being skipped, which is the intended behaviour: a silent skip would report
green for work nobody ran.

## Running the live suite

Build the ELF on a Linux target, then point the suite at it from Windows:

    cargo build --release --manifest-path core/Cargo.toml --bin codotheca-worker
    set CODOTHECA_WORKER_ELF=<path to the built codotheca-worker>
    cargo test --manifest-path core/Cargo.toml --test wsl_live -- --ignored --nocapture

The suite uses a distro that is **already running** and never starts a stopped one: starting a
distro is a consented action, and a test is not where consent is granted.

## Where the shipped copy comes from

CI builds the worker on the Linux runner for both `linux-x64` and `linux-arm64`, because WSL2
runs on arm64 Windows as well; the Windows packaging job downloads them and stages them into the
installer. The two target triples are therefore not independent jobs — the Windows job `needs`
the Linux one.

## Where it lands inside a distro

`$HOME/.cache/codotheca/worker/<fingerprint>/codotheca-worker`, where `<fingerprint>` is a hash
of the binary's own bytes. Nothing outside that directory is written. When the fingerprint
changes, the previous directory is removed with `rmdir`, which refuses a non-empty directory —
so anything that is not exactly what Codotheca put there survives.

## Which distros may be started

A distro that is already running is attached to without asking: doing so starts nothing. A
stopped distro is started only if the user has agreed, and that agreement is a JSON list of
distro names under the `wsl_consented_distros` key in `app_meta`. An unreadable or malformed
value consents to nothing, so the failure mode is to ask again rather than to start a virtual
machine nobody asked for.
