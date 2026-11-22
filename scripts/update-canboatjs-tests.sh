#!/usr/bin/env bash
# Download / update canboatjs per-PGN test fixtures.
#
# canboatjs keeps its decoder tests in `test/pgns/<pgn>.js`. Each file exports
# an array of { input, expected, ... } cases. This script converts them to a
# single JSON file that `crates/nmea2000-pgns/tests/canboatjs.rs` reads.
#
# Requirements: git, node (only for this one-time conversion step).
#
# Usage:
#   ./scripts/update-canboatjs-tests.sh [git-ref]   # default: master

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$SCRIPT_DIR/../data/conformance/canboatjs"
REF="${1:-master}"

command -v node >/dev/null || { echo "node is required for the conversion step" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

git clone -q --depth 1 --branch "$REF" --filter=blob:none --sparse \
    https://github.com/canboat/canboatjs.git "$WORK/canboatjs"
git -C "$WORK/canboatjs" sparse-checkout set test/pgns >/dev/null
COMMIT="$(git -C "$WORK/canboatjs" rev-parse --short=12 HEAD)"

mkdir -p "$DATA_DIR"
node -e '
const fs = require("fs");
const path = require("path");
const dir = process.argv[1];
const files = fs.readdirSync(dir).filter((f) => f.endsWith(".js")).sort();
const out = [];
for (const file of files) {
  const cases = require(path.join(dir, file));
  cases.forEach((c, index) => out.push({ file, index, ...c }));
}
process.stdout.write(JSON.stringify(out, null, 1) + "\n");
' "$WORK/canboatjs/test/pgns" > "$DATA_DIR/pgns.json"

echo "$COMMIT  $(date -Iseconds)" > "$DATA_DIR/UPSTREAM_VERSION"

echo "canboatjs commit: $COMMIT"
echo "Cases: $(grep -c '"file"' "$DATA_DIR/pgns.json")"
echo "Saved to: $DATA_DIR/pgns.json"
