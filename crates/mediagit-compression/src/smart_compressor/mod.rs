// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

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
