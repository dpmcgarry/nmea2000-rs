#!/usr/bin/env bash
# Conformance check: compare canboat analyzer output with our library.
#
# Requirements:
#   - canboat `analyzer` binary in PATH
#   - Rust toolchain (for cargo test)
#
# This script:
#   1. Runs canboat analyzer on test vectors → reference JSON
#   2. Runs our Rust conformance tests
#   3. Runs performance comparison (decode throughput)
#
# Exit code: 0 if all checks pass, 1 if any fail.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DATA_DIR="$PROJECT_DIR/data/conformance"
TEST_INPUT="$DATA_DIR/pgn-test.in"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

pass=0
fail=0

section() { echo -e "\n${YELLOW}=== $1 ===${NC}"; }
ok()      { echo -e "  ${GREEN}PASS${NC}: $1"; pass=$((pass + 1)); }
err()     { echo -e "  ${RED}FAIL${NC}: $1"; fail=$((fail + 1)); }

# ─── Step 1: Verify canboat analyzer is available ─────────────────────────

section "Environment"

if command -v analyzer &>/dev/null; then
    ANALYZER_VERSION=$(analyzer -version 2>&1 | head -1 || echo "unknown")
    ok "canboat analyzer found: $ANALYZER_VERSION"
else
    err "canboat analyzer not found in PATH"
    echo "  Install canboat: git clone https://github.com/canboat/canboat && cd canboat && make"
    exit 1
fi

# ─── Step 2: Run canboat analyzer on test vectors ─────────────────────────

section "Canboat Reference Output"

CANBOAT_JSON=$(mktemp)
if analyzer -json < "$TEST_INPUT" > "$CANBOAT_JSON" 2>/dev/null; then
    CANBOAT_LINES=$(wc -l < "$CANBOAT_JSON")
    ok "canboat decoded $CANBOAT_LINES messages"
else
    err "canboat analyzer failed"
fi

# Verify canboat JSON is valid
if python3 -c "
import json, sys
with open('$CANBOAT_JSON') as f:
    for i, line in enumerate(f, 1):
        json.loads(line)
print(f'{i} valid JSON lines')
" 2>/dev/null; then
    ok "canboat JSON output is valid"
else
    err "canboat JSON output is malformed"
fi

# ─── Step 3: Run our conformance tests ────────────────────────────────────

section "Our Library Conformance Tests"

cd "$PROJECT_DIR"
if cargo test --package nmea2000-pgns --test conformance 2>&1 | tail -5; then
    ok "all conformance tests passed"
else
    err "some conformance tests failed"
fi

# ─── Step 4: Performance comparison ───────────────────────────────────────

section "Performance Comparison"

# Generate a larger test corpus (10000 lines) by repeating test vectors
LARGE_CORPUS=$(mktemp)
for i in $(seq 1 600); do
    grep -v '^#' "$TEST_INPUT" | grep -v '^$'
done > "$LARGE_CORPUS"
CORPUS_LINES=$(wc -l < "$LARGE_CORPUS")
echo "  Test corpus: $CORPUS_LINES messages"

# Time canboat analyzer
CANBOAT_START=$(date +%s%N)
analyzer -json < "$LARGE_CORPUS" > /dev/null 2>&1
CANBOAT_END=$(date +%s%N)
CANBOAT_MS=$(( (CANBOAT_END - CANBOAT_START) / 1000000 ))
CANBOAT_RATE=$(( CORPUS_LINES * 1000 / (CANBOAT_MS + 1) ))
echo "  canboat analyzer: ${CANBOAT_MS}ms for $CORPUS_LINES messages (${CANBOAT_RATE} msg/s)"

# Run criterion benchmark for our library
echo ""
echo "  Our library (criterion):"
cargo bench --package nmea2000-pgns --bench throughput_bench 2>&1 | grep -E "time:|throughput" | head -4

# Cleanup
rm -f "$CANBOAT_JSON" "$LARGE_CORPUS"

# ─── Summary ──────────────────────────────────────────────────────────────

section "Summary"
echo -e "  ${GREEN}Passed${NC}: $pass"
if [ $fail -gt 0 ]; then
    echo -e "  ${RED}Failed${NC}: $fail"
    exit 1
else
    echo -e "  ${RED}Failed${NC}: 0"
    echo -e "  ${GREEN}All checks passed!${NC}"
fi
