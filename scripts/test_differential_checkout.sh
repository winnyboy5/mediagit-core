#!/bin/bash
# Test Differential Checkout Performance
# Measures the benefit of skipping unchanged files on branch switches

set -e

# Store current directory
ORIG_DIR=$(pwd)
MEDIAGIT_BIN="$ORIG_DIR/target/release/mediagit"
TEST_REPO="/tmp/mediagit-diff-checkout-test-$$"

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${BLUE}╔═══════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║  Differential Checkout Performance Test      ║${NC}"
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

echo -e "${BLUE}📦 Setting up test repository with real media files...${NC}"

# Add multiple files
cp "$TEST_FILES/wooden-table.glb" . 2>/dev/null || echo "Skipping wooden-table.glb"
cp "$TEST_FILES/wooden-table.usdz" . 2>/dev/null || echo "Skipping wooden-table.usdz"
cp "$TEST_FILES/sample-3d-model.stl" . 2>/dev/null || echo "Skipping sample-3d-model.stl"
cp "$TEST_FILES/vintage-camera.glb" . 2>/dev/null || echo "Skipping vintage-camera.glb"
cp "$TEST_FILES/sample.jpg" . 2>/dev/null || echo "Skipping sample.jpg"
cp "$TEST_FILES/test.png" . 2>/dev/null || echo "Skipping test.png"

# Find any video files
VIDEO_FILE=$(find "$TEST_FILES" -type f -name "*.mp4" -print -quit 2>/dev/null || echo "")
if [ -n "$VIDEO_FILE" ]; then
    cp "$VIDEO_FILE" ./video.mp4 2>/dev/null || echo "Skipping video"
fi

$MEDIAGIT_BIN add . >/dev/null 2>&1
$MEDIAGIT_BIN commit -m "Initial commit with media files" >/dev/null 2>&1

echo -e "${GREEN}✅ Created initial commit${NC}"

# Create feature branch and add one new file
$MEDIAGIT_BIN branch create feature-branch >/dev/null 2>&1
$MEDIAGIT_BIN branch switch feature-branch >/dev/null 2>&1

echo "New feature file" > feature.txt
$MEDIAGIT_BIN add feature.txt >/dev/null 2>&1
$MEDIAGIT_BIN commit -m "Add feature file" >/dev/null 2>&1

echo -e "${GREEN}✅ Created feature branch with additional file${NC}"
echo ""

# Test 1: First switch from feature to main (all files need checkout)
echo -e "${BLUE}═══ Test 1: First switch (main → feature) ═══${NC}"
echo "  Expected: Slower (all files written)"
START=$(date +%s%3N)
$MEDIAGIT_BIN branch switch main 2>&1 | grep -v "INFO" >/dev/null
END=$(date +%s%3N)
FIRST_SWITCH_TIME=$((END - START))
echo -e "${YELLOW}  First switch time: ${FIRST_SWITCH_TIME}ms${NC}"

# Test 2: Switch back to feature (differential should help)
echo ""
echo -e "${BLUE}═══ Test 2: Second switch (main → feature) ═══${NC}"
echo "  Expected: Faster (most files unchanged, skipped)"
START=$(date +%s%3N)
$MEDIAGIT_BIN branch switch feature-branch 2>&1 | grep -v "INFO" >/dev/null
END=$(date +%s%3N)
SECOND_SWITCH_TIME=$((END - START))
echo -e "${YELLOW}  Second switch time: ${SECOND_SWITCH_TIME}ms${NC}"

# Test 3: Switch back to main again (should be fast)
echo ""
echo -e "${BLUE}═══ Test 3: Third switch (feature → main) ═══${NC}"
echo "  Expected: Fastest (all files already correct)"
START=$(date +%s%3N)
$MEDIAGIT_BIN branch switch main 2>&1 | grep -v "INFO" >/dev/null
END=$(date +%s%3N)
THIRD_SWITCH_TIME=$((END - START))
echo -e "${YELLOW}  Third switch time: ${THIRD_SWITCH_TIME}ms${NC}"

# Test 4: Multiple rapid switches
echo ""
echo -e "${BLUE}═══ Test 4: Rapid switching (10 switches) ═══${NC}"
TOTAL_TIME=0
for i in {1..5}; do
    START=$(date +%s%3N)
    $MEDIAGIT_BIN branch switch feature-branch 2>&1 | grep -v "INFO" >/dev/null
    END=$(date +%s%3N)
    SWITCH_TIME=$((END - START))
    TOTAL_TIME=$((TOTAL_TIME + SWITCH_TIME))

    START=$(date +%s%3N)
    $MEDIAGIT_BIN branch switch main 2>&1 | grep -v "INFO" >/dev/null
    END=$(date +%s%3N)
    SWITCH_TIME=$((END - START))
    TOTAL_TIME=$((TOTAL_TIME + SWITCH_TIME))
done
AVERAGE_TIME=$((TOTAL_TIME / 10))
echo -e "${YELLOW}  Average switch time: ${AVERAGE_TIME}ms${NC}"

# Calculate improvement
echo ""
echo -e "${BLUE}╔═══════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           Performance Results                 ║${NC}"
echo -e "${BLUE}╚═══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "First switch (cold):     ${FIRST_SWITCH_TIME}ms"
echo -e "Second switch:           ${SECOND_SWITCH_TIME}ms"
echo -e "Third switch:            ${THIRD_SWITCH_TIME}ms"
echo -e "Average (10 switches):   ${AVERAGE_TIME}ms"
echo ""

if [ $AVERAGE_TIME -lt $FIRST_SWITCH_TIME ]; then
    IMPROVEMENT=$(awk "BEGIN {printf \"%.1f\", ($FIRST_SWITCH_TIME - $AVERAGE_TIME) * 100 / $FIRST_SWITCH_TIME}")
    echo -e "${GREEN}✅ Optimization effective: ${IMPROVEMENT}% faster on average${NC}"
else
    echo -e "${YELLOW}⚠️  Average not faster than first switch${NC}"
fi

if [ $THIRD_SWITCH_TIME -lt 200 ]; then
    echo -e "${GREEN}✅ Subsequent switches meeting target (<200ms)${NC}"
else
    echo -e "${YELLOW}⚠️  Subsequent switches: ${THIRD_SWITCH_TIME}ms (target <200ms)${NC}"
fi

echo ""
echo -e "${BLUE}═══ Differential Checkout Analysis ═══${NC}"
echo ""
echo "The differential checkout optimization:"
echo "1. Checks file size before reading (cheap)"
echo "2. Computes hash only if size matches (fast path)"
echo "3. Skips write if hash matches (saves I/O)"
echo ""
echo "Expected behavior:"
echo "- First switch: Slower (files don't exist, no skipping)"
echo "- Subsequent switches: Faster (files unchanged, skipped)"
echo "- Best case: 70-90% reduction when most files unchanged"
echo ""
echo -e "${GREEN}Test complete!${NC}"
