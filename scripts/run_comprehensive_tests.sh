#!/bin/bash
# Comprehensive E2E Test Runner for MediaGit
# Runs all test suites with proper categorization and reporting

set -e

# Colors for output
RED='\033[0[31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0;m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}MediaGit Comprehensive Test Suite${NC}"
echo -e "${BLUE}========================================${NC}\n"

# Test results tracking
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

# Function to run test suite
run_test_suite() {
    local suite_name=$1
    local test_command=$2

    echo -e "\n${YELLOW}Running: $suite_name${NC}"
    echo "Command: $test_command"
    echo "----------------------------------------"

    if eval "$test_command"; then
        echo -e "${GREEN}✓ $suite_name PASSED${NC}"
        ((PASSED_TESTS++))
    else
        echo -e "${RED}✗ $suite_name FAILED${NC}"
        ((FAILED_TESTS++))
    fi
    ((TOTAL_TESTS++))
}

# Phase 1: Unit Tests
echo -e "\n${BLUE}=== PHASE 1: Unit Tests ===${NC}"

run_test_suite "Progress Module Tests" "cargo test --package mediagit-cli --lib progress"
run_test_suite "Compression Tests" "cargo test --package mediagit-compression"
run_test_suite "Storage Tests" "cargo test --package mediagit-storage --lib"
run_test_suite "Versioning Tests" "cargo test --package mediagit-versioning --lib"
run_test_suite "Config Tests" "cargo test --package mediagit-config"

# Phase 2: Integration Tests (Fast)
echo -e "\n${BLUE}=== PHASE 2: Integration Tests (Fast) ===${NC}"

run_test_suite "CLI Command Tests" "cargo test --package mediagit-cli --test cli_command_tests"
run_test_suite "Init Command Tests" "cargo test --package mediagit-cli --test cmd_init_tests"
run_test_suite "Add Command Tests" "cargo test --package mediagit-cli --test cmd_add_tests"
run_test_suite "Commit Command Tests" "cargo test --package mediagit-cli --test cmd_commit_tests"
run_test_suite "Branch Command Tests" "cargo test --package mediagit-cli --test cmd_branch_tests"
run_test_suite "Status Command Tests" "cargo test --package mediagit-cli --test cmd_status_tests"

# Phase 3: E2E Tests (Small Files)
echo -e "\n${BLUE}=== PHASE 3: E2E Tests (Small Files) ===${NC}"

run_test_suite "Basic Workflow Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_basic_workflow"
run_test_suite "Media File Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_3d"
run_test_suite "Media Audio Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_audio"
run_test_suite "Mixed Format Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_mixed"
run_test_suite "Branch Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_branch"
run_test_suite "Multiple Files Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_multiple"
run_test_suite "File Modification Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_file_modification"
run_test_suite "Error Handling Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_add_nonexistent"
run_test_suite "Progress Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_verify"

# Phase 4: E2E Tests (Large Files - Optional)
echo -e "\n${YELLOW}=== PHASE 4: Large File Tests (Optional - run with --include-large) ===${NC}"

if [[ "$1" == "--include-large" ]]; then
    echo "Running large file tests..."
    run_test_suite "Large File 264MB" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_large_file_video_264mb -- --ignored"
    run_test_suite "Large File 398MB" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_large_file_video_398mb -- --ignored"

    if [[ "$2" == "--include-huge" ]]; then
        run_test_suite "Very Large File 2GB" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_very_large_file_2gb -- --ignored"
    else
        echo -e "${YELLOW}Skipping 2GB test (use --include-huge to run)${NC}"
        ((SKIPPED_TESTS++))
    fi
else
    echo -e "${YELLOW}Skipping large file tests (use --include-large to run)${NC}"
    SKIPPED_TESTS=3
fi

# Phase 5: Server Tests (if enabled)
echo -e "\n${BLUE}=== PHASE 5: Server Tests ===${NC}"

if command -v docker &> /dev/null; then
    run_test_suite "Server Integration" "cargo test --package mediagit-server --test server_tests"
    run_test_suite "Auth Tests" "cargo test --package mediagit-server --test auth_integration_tests"
else
    echo -e "${YELLOW}Docker not available, skipping server tests${NC}"
    SKIPPED_TESTS=$((SKIPPED_TESTS + 2))
fi

# Summary
echo -e "\n${BLUE}========================================${NC}"
echo -e "${BLUE}Test Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "Total Suites: $TOTAL_TESTS"
echo -e "${GREEN}Passed: $PASSED_TESTS${NC}"
echo -e "${RED}Failed: $FAILED_TESTS${NC}"
echo -e "${YELLOW}Skipped: $SKIPPED_TESTS${NC}"

if [ $FAILED_TESTS -eq 0 ]; then
    echo -e "\n${GREEN}✓ All tests passed!${NC}"
    exit 0
else
    echo -e "\n${RED}✗ Some tests failed${NC}"
    exit 1
fi
