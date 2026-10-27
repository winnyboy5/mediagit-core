// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Generates Tauri context (config, capabilities, icons) at compile time.

fn main() {
    tauri_build::build();
}
