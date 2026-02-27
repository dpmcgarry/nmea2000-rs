#!/usr/bin/env bash
# Download / update canboat test vectors from upstream.
#
# This fetches the official canboat analyzer test inputs and reference outputs.
# Re-run whenever canboat updates its test suite.
#
# Usage:
#   ./scripts/update-canboat-tests.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$SCRIPT_DIR/../data/conformance"

BASE_URL="https://raw.githubusercontent.com/canboat/canboat/master/analyzer/tests"

# Files to download: input files and their expected outputs
FILES=(
    # Primary test suite
    "pgn-test.in"
    "pgn-test.out"
    "pgn-test-json-nv.out"
    "pgn-test-json-nv.err"
    # Actisense format
    "pgn-test-actisense.in"
    "pgn-test-actisense.out"
    # Recombine/fast-packet test
    "recombine-frames.in"
    "recombine-frames.out"
    # Mixed format
    "mixed-format.in"
    "mixed-format.out"
    # DMS coordinate format
    "dms-format.in"
    "dms-format.out"
    # Alert PGN with nested ISO_NAME
    "pgn-126983.in"
    "pgn-126983-nv.out"
    # Seatalk pilot mode
    "pgn-65379-test.in"
    "pgn-65379-test.out"
    # J1939
    "j1939-pgn-test.in"
    "j1939-pgn-test.out"
)

mkdir -p "$DATA_DIR"

echo "Downloading canboat test files to $DATA_DIR ..."
for file in "${FILES[@]}"; do
    echo "  $file"
    curl -fsSL "$BASE_URL/$file" -o "$DATA_DIR/$file" || {
        echo "  WARNING: failed to download $file (may not exist upstream)"
    }
done

# Record the commit hash for traceability
COMMIT=$(curl -fsSL "https://api.github.com/repos/canboat/canboat/commits/master" \
    | grep -m1 '"sha"' | cut -d'"' -f4 | head -c 12)
echo "$COMMIT  $(date -Iseconds)" > "$DATA_DIR/UPSTREAM_VERSION"

echo ""
echo "Done. Canboat commit: $COMMIT"
echo "Files saved to: $DATA_DIR/"
