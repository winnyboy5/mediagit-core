#!/bin/bash
# Test Parallel Status Performance
# Measures the benefit of parallel file hashing for status command

set -e

# Store current directory
ORIG_DIR=$(pwd)
MEDIAGIT_BIN="$ORIG_DIR/target/release/mediagit"
TEST_REPO="/tmp/mediagit-parallel-status-test-$$"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}╔═══════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║  Parallel Status Performance Test            ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════╝${NC}"
echo ""

cleanup() {
    echo -e "\n${YELLOW}🧹 Cleaning up...${NC}"
    rm -rf "$TEST_REPO"
}
trap cleanup EXIT

# Initialize repo
mkdir -p "$TEST_REPO"
cd "$TEST_REPO"
$MEDIAGIT_BIN init >/dev/null 2>&1

# Get test files
TEST_FILES="$ORIG_DIR/test-files"
if [ ! -d "$TEST_FILES" ]; then
    echo "Test files not found at $TEST_FILES"
    exit 1
fi

echo -e "${BLUE}📦 Setting up test repository with media files...${NC}"

# Add all available test files
cp "$TEST_FILES"/*.glb . 2>/dev/null || true
cp "$TEST_FILES"/*.usdz . 2>/dev/null || true
cp "$TEST_FILES"/*.stl . 2>/dev/null || true
cp "$TEST_FILES"/*.jpg . 2>/dev/null || true
cp "$TEST_FILES"/*.png . 2>/dev/null || true

# Find and add any video files
find "$TEST_FILES" -type f -name "*.mp4" -exec cp {} . \; 2>/dev/null || true

FILE_COUNT=$(ls -1 | wc -l)
echo -e "${GREEN}✅ Copied $FILE_COUNT test files${NC}"

$MEDIAGIT_BIN add . >/dev/null 2>&1
$MEDIAGIT_BIN commit -m "Initial commit with $FILE_COUNT files" >/dev/null 2>&1

echo -e "${GREEN}✅ Created initial commit${NC}"
echo ""

# Test 1: Status on clean repository (baseline)
echo -e "${BLUE}═══ Test 1: Clean Status (Baseline) ═══${NC}"
echo "  All files match HEAD"
START=$(date +%s%3N)
$MEDIAGIT_BIN status 2>&1 | grep -v "INFO" >/dev/null
END=$(date +%s%3N)
CLEAN_STATUS_TIME=$((END - START))
echo -e "${YELLOW}  Clean status time: ${CLEAN_STATUS_TIME}ms${NC}"

# Test 2: Status after modifying one file
echo ""
echo -e "${BLUE}═══ Test 2: Status with 1 Modified File ═══${NC}"
FIRST_FILE=$(ls *.glb 2>/dev/null | head -1 || ls *.jpg 2>/dev/null | head -1)
if [ -n "$FIRST_FILE" ]; then
    echo "Modified file" >> "$FIRST_FILE"
    START=$(date +%s%3N)
    $MEDIAGIT_BIN status 2>&1 | grep -v "INFO" >/dev/null
    END=$(date +%s%3N)
    ONE_MODIFIED_TIME=$((END - START))
    echo -e "${YELLOW}  Status time (1 modified): ${ONE_MODIFIED_TIME}ms${NC}"
    git restore "$FIRST_FILE" 2>/dev/null || true
fi

# Test 3: Status after modifying half the files
echo ""
echo -e "${BLUE}═══ Test 3: Status with Multiple Modified Files ═══${NC}"
MODIFY_COUNT=$((FILE_COUNT / 2))
echo "  Modifying $MODIFY_COUNT files..."

# Modify half the files
COUNT=0
for file in *.glb *.jpg *.png; do
    if [ -f "$file" ] && [ $COUNT -lt $MODIFY_COUNT ]; then
        echo "Modified" >> "$file"
        COUNT=$((COUNT + 1))
    fi
done 2>/dev/null

START=$(date +%s%3N)
$MEDIAGIT_BIN status 2>&1 | grep -v "INFO" >/dev/null
END=$(date +%s%3N)
MULTI_MODIFIED_TIME=$((END - START))
echo -e "${YELLOW}  Status time ($COUNT modified): ${MULTI_MODIFIED_TIME}ms${NC}"

# Test 4: Multiple status calls (average)
echo ""
echo -e "${BLUE}═══ Test 4: Average Status Time (5 calls) ═══${NC}"
TOTAL_TIME=0
for i in {1..5}; do
    START=$(date +%s%3N)
    $MEDIAGIT_BIN status 2>&1 | grep -v "INFO" >/dev/null
    END=$(date +%s%3N)
    STATUS_TIME=$((END - START))
    TOTAL_TIME=$((TOTAL_TIME + STATUS_TIME))
done
AVERAGE_TIME=$((TOTAL_TIME / 5))
echo -e "${YELLOW}  Average status time: ${AVERAGE_TIME}ms (with $COUNT modified files)${NC}"

# Results summary
echo ""
echo -e "${BLUE}╔═══════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           Performance Results                 ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "Test repository: $FILE_COUNT files"
echo ""
echo -e "Clean status:            ${CLEAN_STATUS_TIME}ms"
echo -e "1 file modified:         ${ONE_MODIFIED_TIME}ms"
echo -e "$COUNT files modified:     ${MULTI_MODIFIED_TIME}ms"
echo -e "Average (5 calls):       ${AVERAGE_TIME}ms"
echo ""

# Performance assessment
if [ $AVERAGE_TIME -lt 800 ]; then
    echo -e "${GREEN}✅ Meeting target (<800ms)${NC}"
elif [ $AVERAGE_TIME -lt 1500 ]; then
    echo -e "${YELLOW}⚠️  Acceptable performance (800-1500ms)${NC}"
else
    echo -e "${YELLOW}⚠️  Above target (>${AVERAGE_TIME}ms)${NC}"
fi

# Multi-core utilization check
echo ""
echo -e "${BLUE}═══ Parallel Optimization Analysis ═══${NC}"
echo ""
echo "The parallel status optimization:"
echo "1. Uses Rayon for multi-core file hashing"
echo "2. Avoids async overhead with direct Oid::hash()"
echo "3. Processes files in parallel across all CPU cores"
echo ""
echo "Expected behavior:"
echo "- Clean repo: Fast (all files match HEAD)"
echo "- Modified files: Parallel hashing shows benefit"
echo "- Scales with CPU core count (2-8x speedup)"
echo ""
echo -e "${GREEN}Test complete!${NC}"
