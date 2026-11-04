// SPDX-License-Identifier: BUSL-1.1
// Copyright (c) 2026 MediaGit Contributors

//! FUTURE_TODOS #38: a delta's `result_size` comes off disk or the wire and
//! must not decide how much memory the decoder grabs. A forged size with a
//! tiny payload has to fail without a matching allocation.
//!
//! Own test binary: the counting allocator is global, and other tests in the
//! same process would pollute the peak.

// A `GlobalAlloc` impl is unsafe by definition; it only forwards to `System`.
#![allow(unsafe_code)]

use mediagit_versioning::{Delta, DeltaDecoder, DeltaEncoder};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Largest;

static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LARGEST.fetch_max(new_size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Largest = Largest;

const GIB: usize = 1024 * 1024 * 1024;

/// Forge, on the wire, both sizes an attacker controls: `result_size` and the
/// zstd frame's own content-size field (the zstd crate sizes its buffer from
/// the frame header). The frame carries one real byte.
fn forged(header: &[u8], base_size: usize, claimed: usize) -> Delta {
    let mut bytes = header.to_vec();
    varint(&mut bytes, base_size as u64);
    varint(&mut bytes, claimed as u64);
    bytes.extend_from_slice(&[0x28, 0xB5, 0x2F, 0xFD]); // zstd magic
    bytes.push(0xE0); // single segment, 8-byte frame content size
    bytes.extend_from_slice(&(claimed as u64).to_le_bytes());
    bytes.extend_from_slice(&[0x0B, 0x00, 0x00, b'A']); // last block, RLE, 1 byte
    Delta::from_bytes(&bytes).unwrap()
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[test]
fn forged_result_size_does_not_drive_the_allocation() {
    let base = vec![7u8; 4096];
    let headers: [(&str, &[u8]); 2] =
        [("zstd-dict", b"ZD"), ("xor-planes", &[b'X', b'P', 1, 2, 0])];

    for (name, header) in headers {
        let delta = forged(header, base.len(), 4 * GIB);
        LARGEST.store(0, Ordering::Relaxed);
        let err = DeltaDecoder::apply(&base, &delta);
        assert!(
            err.is_err(),
            "{name}: a one-byte frame cannot honour a 4 GiB claim"
        );
        let peak = LARGEST.load(Ordering::Relaxed);
        assert!(
            peak < 128 * 1024 * 1024,
            "{name}: decoder allocated {peak} bytes for a forged 4 GiB result_size"
        );
    }
}

#[test]
fn large_real_delta_still_round_trips() {
    // Above the pre-allocation limit the decoder grows with the decoded bytes;
    // a genuine large delta must still reconstruct exactly.
    let n = 80 * 1024 * 1024;
    let base: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
    let mut target = base.clone();
    for i in (0..n).step_by(4099) {
        target[i] ^= 0x5a;
    }
    let delta = DeltaEncoder::encode(&base, &target);
    assert_eq!(DeltaDecoder::apply(&base, &delta).unwrap(), target);
    let delta = DeltaEncoder::encode_xor_planes(&base, &target, 1, 0);
    assert_eq!(DeltaDecoder::apply(&base, &delta).unwrap(), target);
}
