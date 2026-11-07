#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
npm ci
npm run build
cargo build --release -p stufftracker
python3 scripts/pack-oci.py target/release/stufftracker img.tar
