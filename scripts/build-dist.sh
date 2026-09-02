#!/usr/bin/env bash
# Thin wrapper. All the logic lives in build-dist.mjs so the two platforms cannot drift apart.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if ! command -v node >/dev/null 2>&1; then
        echo "build-dist: node is not on PATH; install Node 22.12.0 or newer" >&2
        exit 1
fi

exec node "$here/build-dist.mjs" "$@"
