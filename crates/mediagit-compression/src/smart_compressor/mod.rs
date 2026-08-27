// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Smart compression with per-object-type strategy selection
//!
//! Automatically selects optimal compression based on file type and content.

use crate::error::CompressionResult;
use crate::{BrotliCompressor, CompressionLevel, Compressor, ZlibCompressor, ZstdCompressor};
use std::fmt;
use std::path::Path;

pub(crate) mod compressor;
pub(crate) mod object_type;
pub(crate) mod strategy;

pub use compressor::*;
pub use object_type::*;
pub use strategy::*;
