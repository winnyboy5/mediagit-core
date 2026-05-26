#!/bin/bash
# Comprehensive E2E Test Runner for MediaGit
# Runs all test suites with proper categorization and reporting

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}MediaGit Comprehensive Test Suite${NC}"
echo -e "${BLUE}========================================${NC}\n"

# Test results tracking
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

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

run_test_suite "Compression Tests"  "cargo test --package mediagit-compression"
run_test_suite "Storage Tests"      "cargo test --package mediagit-storage --lib"
run_test_suite "Versioning Tests"   "cargo test --package mediagit-versioning --lib"
run_test_suite "Config Tests"       "cargo test --package mediagit-config --lib"
run_test_suite "Git Filter Tests"   "cargo test --package mediagit-git --lib"
run_test_suite "Protocol Tests"     "cargo test --package mediagit-protocol --lib"

# Phase 2: CLI Integration Tests
echo -e "\n${BLUE}=== PHASE 2: CLI Integration Tests ===${NC}"

run_test_suite "CLI Command Tests"  "cargo test --package mediagit-cli --test cli_command_tests"
run_test_suite "Init Tests"         "cargo test --package mediagit-cli --test cli_init_test"
run_test_suite "Add Tests"          "cargo test --package mediagit-cli --test cli_add_test"
run_test_suite "Commit Tests"       "cargo test --package mediagit-cli --test cli_commit_test"
run_test_suite "Branch Tests"       "cargo test --package mediagit-cli --test cli_branch_test"
run_test_suite "Merge Tests"        "cargo test --package mediagit-cli --test cli_merge_test"
run_test_suite "Status/Log Tests"   "cargo test --package mediagit-cli --test cli_status_log_test"
run_test_suite "Tag Tests"          "cargo test --package mediagit-cli --test cli_tag_test"
run_test_suite "Stash Tests"        "cargo test --package mediagit-cli --test cli_stash_test"
run_test_suite "Reset Tests"        "cargo test --package mediagit-cli --test cli_reset_test"
run_test_suite "Revert Tests"       "cargo test --package mediagit-cli --test cli_revert_test"
run_test_suite "Rebase/Cherrypick" "cargo test --package mediagit-cli --test cli_rebase_cherrypick_test"
run_test_suite "Reflog Tests"       "cargo test --package mediagit-cli --test cli_reflog_test"
run_test_suite "Remote Tests"       "cargo test --package mediagit-cli --test cli_remote_test"
run_test_suite "Maintenance Tests"  "cargo test --package mediagit-cli --test cli_maintenance_test"
run_test_suite "Log Format Tests"   "cargo test --package mediagit-cli --test cli_log_format_test"
run_test_suite "Advanced Tests"     "cargo test --package mediagit-cli --test cli_advanced_test"
run_test_suite "Ignore Tests"       "cargo test --package mediagit-cli --test ignore_integration_test"
run_test_suite "Progress Tests"     "cargo test --package mediagit-cli --test progress_test"

# Phase 3: E2E Tests (Small Files)
echo -e "\n${BLUE}=== PHASE 3: E2E Tests (Small Files) ===${NC}"

run_test_suite "Basic Workflow"     "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_basic_workflow"
run_test_suite "Media 3D Tests"     "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_3d"
run_test_suite "Media Audio Tests"  "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_audio"
run_test_suite "Mixed Format Tests" "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_media_mixed"
run_test_suite "Branch Tests"       "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_branch"
run_test_suite "Multiple Files"     "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_multiple"
run_test_suite "File Modification"  "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_file_modification"
run_test_suite "Error Handling"     "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_add_nonexistent"
run_test_suite "Verify Tests"       "cargo test -p mediagit-cli --test comprehensive_e2e_tests e2e_verify"

# Phase 4: E2E Tests (Large Files - Optional)
echo -e "\n${YELLOW}=== PHASE 4: Large File Tests (Optional) ===${NC}"

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

# Phase 5: Server Integration Tests
echo -e "\n${BLUE}=== PHASE 5: Server Integration Tests ===${NC}"

run_test_suite "Server Tests"           "cargo test --package mediagit-server --test server_tests"
run_test_suite "Auth Integration"       "cargo test --package mediagit-server --test auth_integration_tests"
run_test_suite "E2E Push/Pull"          "cargo test --package mediagit-server --test e2e_push_pull"
run_test_suite "E2E Push/Pull Tags"     "cargo test --package mediagit-server --test e2e_push_pull_tags"
run_test_suite "E2E Non-FF Push"        "cargo test --package mediagit-server --test e2e_non_ff_push"
run_test_suite "E2E Incremental Fetch"  "cargo test --package mediagit-server --test e2e_incremental_fetch"
run_test_suite "E2E Delta Preservation" "cargo test --package mediagit-server --test e2e_push_preserves_deltas"
run_test_suite "E2E Presigned Download" "cargo test --package mediagit-server --test e2e_presigned_download"
run_test_suite "E2E Presigned Upload"   "cargo test --package mediagit-server --test e2e_presigned_upload"

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
