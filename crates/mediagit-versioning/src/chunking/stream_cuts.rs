// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! E2 stream-boundary chunking (default on; `MEDIAGIT_STREAM_CUTS=0` turns it off).
//!
//! For PDF-family (`pdf`, `ai`) and ZIP-family (`fig`, `sketch`, `xd`, `idml`)
//! containers, forces chunk cuts at the structural boundaries of embedded
//! streams (PDF `stream` data starts / `endstream`; ZIP local headers) so an
//! edited stream no longer shifts chunk boundaries of its neighbours. Segments
//! under 64 KiB are merged with a neighbour up to 256 KiB; FastCDC (today's
//! creative params) then runs inside each segment. No format change: chunks are
//! plain `Generic` chunks keyed by BLAKE3 of their bytes.
//!
//! [`chunk_ranges`] is the single function both the in-memory and the mmap
//! streaming path call, so they cut at identical offsets.

use fastcdc::v2020::{FastCDC, Normalization};

const K: usize = 1024;
const MERGE_BELOW: usize = 64 * K;
const MERGE_UP_TO: usize = 256 * K;

static STREAM_CUTS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// On by default; `MEDIAGIT_STREAM_CUTS=0` turns it off. Measured on a real
/// repo (Figma -19%, 30 MB PDF -4%, others unchanged, add ~10% faster).
pub(crate) fn stream_cuts_enabled() -> bool {
    *STREAM_CUTS
        .get_or_init(|| stream_cuts_from(std::env::var("MEDIAGIT_STREAM_CUTS").ok().as_deref()))
}

fn stream_cuts_from(v: Option<&str>) -> bool {
    v.map(str::trim) != Some("0")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Pdf,
    Zip,
}

/// Family for `ext` when `data` carries the matching magic at offset 0.
fn detect(data: &[u8], ext: &str) -> Option<Kind> {
    match ext {
        "pdf" | "ai" if data.starts_with(b"%PDF-") => Some(Kind::Pdf),
        "fig" | "sketch" | "xd" | "idml" if data.starts_with(b"PK\x03\x04") => Some(Kind::Zip),
        _ => None,
    }
}

/// Whether `ext` is in scope (magic is validated separately).
pub(crate) fn is_stream_cut_ext(ext: &str) -> bool {
    matches!(ext, "pdf" | "ai" | "fig" | "sketch" | "xd" | "idml")
}

/// Sorted, deduplicated interior cut offsets.
fn cuts(d: &[u8], kind: Kind) -> Vec<usize> {
    let mut v = vec![];
    let mut i = 0;
    match kind {
        Kind::Zip => {
            while i + 4 <= d.len() {
                if &d[i..i + 4] == b"PK\x03\x04" {
                    v.push(i);
                    i += 4;
                } else {
                    i += 1;
                }
            }
        }
        Kind::Pdf => {
            while i + 6 <= d.len() {
                if &d[i..i + 6] == b"stream" {
                    if i >= 3 && &d[i - 3..i] == b"end" {
                        v.push(i - 3);
                    } else if d.get(i + 6) == Some(&b'\n') {
                        v.push(i + 7);
                    } else if d.get(i + 6) == Some(&b'\r') && d.get(i + 7) == Some(&b'\n') {
                        v.push(i + 8);
                    }
                    i += 6;
                } else {
                    i += 1;
                }
            }
        }
    }
    v.retain(|&c| c > 0 && c < d.len());
    v.sort_unstable();
    v.dedup();
    v
}

/// `(offset, len)` segments between cut points, tiny ones merged (<= 256 KiB).
fn segments(d: &[u8], kind: Kind) -> Vec<(usize, usize)> {
    let mut b = vec![0];
    b.extend(cuts(d, kind));
    b.push(d.len());
    let mut out: Vec<(usize, usize)> = vec![];
    let mut cur: Option<(usize, usize)> = None;
    for w in b.windows(2) {
        let (o, l) = (w[0], w[1] - w[0]);
        match cur {
            Some((co, cl)) if (cl < MERGE_BELOW || l < MERGE_BELOW) && cl + l <= MERGE_UP_TO => {
                cur = Some((co, cl + l))
            }
            Some(c) => {
                out.push(c);
                cur = Some((o, l));
            }
            None => cur = Some((o, l)),
        }
    }
    out.extend(cur);
    out
}

/// Chunk `(offset, len)` ranges covering `data`, or `None` to decline (wrong
/// extension or missing magic). `params` = `(avg, min, max)`.
pub(crate) fn chunk_ranges(
    data: &[u8],
    ext: &str,
    params: (usize, usize, usize),
    seed: u64,
) -> Option<Vec<(usize, usize)>> {
    let kind = detect(data, ext)?;
    let (avg, min, max) = params;
    Some(
        segments(data, kind)
            .into_iter()
            .flat_map(|(o, l)| {
                FastCDC::with_level_and_seed(
                    &data[o..o + l],
                    min,
                    avg,
                    max,
                    Normalization::Level1,
                    seed,
                )
                .map(move |c| (o + c.offset, c.length))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;

    const P: (usize, usize, usize) = (1 << 20, 512 * K, 4 << 20);

    fn rnd(n: usize, seed: u64) -> Vec<u8> {
        let mut s = seed | 1;
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                (s >> 24) as u8
            })
            .collect()
    }

    /// PDF-like file: streams of the given sizes, alternating LF / CRLF after `stream`.
    fn pdf(sizes: &[usize]) -> (Vec<u8>, Vec<usize>) {
        let mut d = b"%PDF-1.7\n".to_vec();
        let mut starts = vec![];
        for (i, &n) in sizes.iter().enumerate() {
            d.extend_from_slice(format!("{i} 0 obj\n<< /Length {n} >>\nstream").as_bytes());
            d.extend_from_slice(if i % 2 == 0 { b"\n" } else { b"\r\n" });
            starts.push(d.len());
            d.extend(rnd(n, i as u64 + 7).into_iter().map(|b| b | 0x80));
            d.extend_from_slice(b"\nendstream\nendobj\n");
        }
        d.extend_from_slice(b"%%EOF\n");
        (d, starts)
    }

    fn bounds(r: &[(usize, usize)]) -> Vec<usize> {
        r.iter().map(|&(o, _)| o).collect()
    }

    #[test]
    fn cuts_at_stream_starts_and_reassembles() {
        let (d, starts) = pdf(&[300 * K, 400 * K, 6 << 20, 300 * K]);
        let r = chunk_ranges(&d, "pdf", P, 0).unwrap();
        let b = bounds(&r);
        for s in &starts {
            assert!(b.contains(s), "stream start {s} is not a chunk boundary");
        }
        let mut re = vec![];
        for &(o, l) in &r {
            assert_eq!(o, re.len());
            re.extend_from_slice(&d[o..o + l]);
        }
        assert_eq!(re, d);
    }

    #[test]
    fn tiny_segments_merge_and_big_ones_split() {
        let (d, _) = pdf(&[1000; 40]);
        let r = chunk_ranges(&d, "ai", P, 0).unwrap();
        assert!(r.iter().all(|&(_, l)| l <= 256 * K));
        assert!(r.len() < 40, "tiny segments must merge, got {}", r.len());
        let (d, _) = pdf(&[10 << 20]);
        let r = chunk_ranges(&d, "pdf", P, 0).unwrap();
        assert!(r.len() >= 3, "big stream must be sub-chunked");
        assert!(r.iter().all(|&(_, l)| l <= P.2));
    }

    #[test]
    fn zip_family_cuts_at_local_headers_and_declines_bad_magic() {
        let mut d = vec![];
        let mut heads = vec![];
        for i in 0..4 {
            heads.push(d.len());
            d.extend_from_slice(b"PK\x03\x04");
            d.extend(rnd(300 * K, i + 3).into_iter().map(|b| b | 0x80));
        }
        let b = bounds(&chunk_ranges(&d, "fig", P, 0).unwrap());
        for h in &heads {
            assert!(b.contains(h));
        }
        assert!(chunk_ranges(&d, "pdf", P, 0).is_none());
        assert!(chunk_ranges(&rnd(1 << 20, 1), "pdf", P, 0).is_none());
        assert!(chunk_ranges(&rnd(1 << 20, 1), "fig", P, 0).is_none());
        assert!(chunk_ranges(&d, "psd", P, 0).is_none());
    }

    fn stream_ranges(d: &[u8], name: &str, ext: &str) -> Vec<(usize, usize)> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, d).unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4096);
        let c = ContentChunker::new(ChunkStrategy::MediaAware);
        assert!(
            c.collect_stream_cuts(&path, d.len() as u64, ext, &tx)
                .unwrap()
        );
        drop(tx);
        let mut v = vec![];
        while let Ok(c) = rx.try_recv() {
            assert_eq!(c.id, Oid::hash(&c.data));
            v.push((c.offset as usize, c.size));
        }
        v
    }

    #[test]
    fn in_memory_and_streaming_cut_identically() {
        let (d, _) = pdf(&[300 * K, 2 << 20, 5 << 20, 80 * K, 600 * K]);
        let c = ContentChunker::new(ChunkStrategy::MediaAware);
        let mem: Vec<_> = c
            .chunk_stream_cuts(&d, "pdf")
            .unwrap()
            .iter()
            .map(|c| (c.offset as usize, c.size))
            .collect();
        assert_eq!(mem, stream_ranges(&d, "a.pdf", "pdf"));
    }

    #[test]
    fn streaming_declines_bad_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, rnd(6 << 20, 2)).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let c = ContentChunker::new(ChunkStrategy::MediaAware);
        assert!(!c.collect_stream_cuts(&path, 6 << 20, "pdf", &tx).unwrap());
    }

    #[test]
    fn knob_parsing_defaults_on() {
        assert!(stream_cuts_from(None));
        assert!(stream_cuts_from(Some("1")));
        assert!(stream_cuts_from(Some("")));
        assert!(!stream_cuts_from(Some("0")));
        assert!(!stream_cuts_from(Some(" 0 ")));
    }

    /// Knob off: `chunk()` for pdf and fig is exactly the pre-E2 creative FastCDC;
    /// knob on: the same input is cut differently (the cuts are live).
    #[tokio::test]
    async fn knob_off_is_identical_to_today() {
        let (pdf_d, _) = pdf(&[300 * K, 2 << 20, 700 * K]);
        let mut fig_d = vec![];
        for i in 0..4 {
            fig_d.extend_from_slice(b"PK\x03\x04");
            fig_d.extend(rnd(300 * K, i + 9).into_iter().map(|b| b | 0x80));
        }
        let c = ContentChunker::new(ChunkStrategy::MediaAware).with_stream_cuts(false);
        let on = ContentChunker::new(ChunkStrategy::MediaAware).with_stream_cuts(true);
        for (d, name) in [(&pdf_d, "x.pdf"), (&fig_d, "x.fig")] {
            let (avg, min, max) = get_creative_chunk_params(d.len() as u64);
            let want: Vec<_> = c
                .chunk_fastcdc(d, avg, min, max)
                .await
                .unwrap()
                .iter()
                .map(|c| (c.offset, c.size, c.id))
                .collect();
            let got: Vec<_> = c
                .chunk(d, name)
                .await
                .unwrap()
                .iter()
                .map(|c| (c.offset, c.size, c.id))
                .collect();
            assert_eq!(got, want, "{name}");
            let cut: Vec<_> = on
                .chunk(d, name)
                .await
                .unwrap()
                .iter()
                .map(|c| (c.offset, c.size, c.id))
                .collect();
            assert_ne!(cut, want, "{name}: stream cuts did not apply");
        }
    }
}
