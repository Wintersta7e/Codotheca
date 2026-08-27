# codotheca-core

The Rust core. Owns the filesystem scan, all `git` invocation, the SQLite index, and card-art
rasterisation. Runs as a child process of the Electron shell and speaks length-prefixed JSON
over stdin/stdout.

It is deliberately the only writer to the database, and the only component that ever sees a
filesystem path — the renderer receives opaque references.

Specification: the phase-1 protocol and data-model sections.
