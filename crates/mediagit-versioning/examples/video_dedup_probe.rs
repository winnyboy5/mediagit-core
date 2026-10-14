// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Phase 0 measurement probe for video-frame chunking (measurement only).
//!
//! For each `variant` it compares against `base` and prints ONE JSON line per
//! measurement mode:
//!
//! * `today` - the real production path (`ContentChunker` MediaAware +
//!   `collect_file_chunks_blocking`). The container-size cap is cached in a
//!   `OnceLock`, so the cap=0 variant (`today_container`) runs in a separate
//!   process with `MEDIAGIT_CONTAINER_CHUNK_CAP_MB=0`.
//! * `ideal` - hand-parsed MOV/MP4 sample tables; every sample blake3-hashed;
//!   new bytes = unique variant samples absent from base + all non-sample
//!   bytes (moov, headers, gaps) of the variant.
//! * `frame` - the original prototype of the frame chunker: one chunk per
//!   video sample (<= 4 MiB pieces), adjacent same-track non-video samples
//!   grouped up to 1 MiB, every other byte in gap chunks (<= 4 MiB).
//!
//! Run (all modes; spawns itself for the two `today` runs):
//!   `cargo run --release -p mediagit-versioning --example video_dedup_probe -- <base> <variant>...`
//! Single mode: add `--mode today|ideal|frame` before the paths.
//!
//! Env: `MEDIAGIT_CDC_SEED` overrides the seed (default 20260716).
//! Peak working set is printed to stderr as `PEAK_RSS_MB`.

use anyhow::{Context, Result, bail};
use mediagit_versioning::Oid;
use mediagit_versioning::chunking::{ChunkStrategy, ContentChunker};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

const MIB: u64 = 1024 * 1024;
const VIDEO_PIECE: u64 = 4 * MIB;
const GAP_PIECE: u64 = 4 * MIB;
const GROUP_MAX: u64 = MIB;

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut mode = "all".to_string();
    if args.first().map(String::as_str) == Some("--mode") {
        mode = args.get(1).cloned().context("--mode needs a value")?;
        args.drain(0..2);
    }
    if args.len() < 2 {
        bail!("usage: video_dedup_probe [--mode today|ideal|frame] <base> <variant>...");
    }
    let base = &args[0];
    let variants = &args[1..];

    match mode.as_str() {
        "today" => run_today(base, variants)?,
        "ideal" => run_ideal(base, variants)?,
        "frame" => run_frame(base, variants)?,
        "all" => {
            // Cap is a process-wide OnceLock: run `today` twice in child processes.
            let exe = std::env::current_exe()?;
            for cap in [None, Some("0")] {
                let mut cmd = std::process::Command::new(&exe);
                cmd.arg("--mode").arg("today").args(&args);
                if let Some(c) = cap {
                    cmd.env("MEDIAGIT_CONTAINER_CHUNK_CAP_MB", c);
                } else {
                    cmd.env_remove("MEDIAGIT_CONTAINER_CHUNK_CAP_MB");
                }
                let st = cmd.status()?;
                if !st.success() {
                    bail!("today child failed: {st}");
                }
            }
            run_ideal(base, variants)?;
            run_frame(base, variants)?;
        }
        m => bail!("unknown mode {m}"),
    }
    if let Some(mb) = peak_rss_mb() {
        eprintln!("PEAK_RSS_MB[{mode}]: {mb:.1}");
    }
    Ok(())
}

fn file_name(p: &str) -> String {
    Path::new(p)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(p)
        .to_string()
}

/// reuse % given new bytes out of total.
fn reuse_pct(new: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * (1.0 - new as f64 / total as f64)
    }
}

// ───────────────────────────── today ─────────────────────────────

struct TodayChunk {
    id: Oid,
    size: u64,
    ctype: String,
    codec: String,
}

fn seed() -> u64 {
    std::env::var("MEDIAGIT_CDC_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20260716)
}

/// Production path: spawn_blocking producer feeding a tokio mpsc channel.
fn chunk_today(rt: &tokio::runtime::Runtime, path: &str) -> Result<Vec<TodayChunk>> {
    rt.block_on(async {
        let chunker = ContentChunker::with_seed(ChunkStrategy::MediaAware, seed());
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let p = path.to_string();
        let h = tokio::task::spawn_blocking(move || chunker.collect_file_chunks_blocking(&p, tx));
        let mut out = Vec::new();
        while let Some(c) = rx.recv().await {
            out.push(TodayChunk {
                id: c.id,
                size: c.size as u64,
                ctype: format!("{:?}", c.chunk_type),
                codec: format!("{:?}", c.codec_hint),
            });
        }
        h.await??;
        Ok(out)
    })
}

fn run_today(base: &str, variants: &[String]) -> Result<()> {
    let label = if std::env::var("MEDIAGIT_CONTAINER_CHUNK_CAP_MB").as_deref() == Ok("0") {
        "today_container"
    } else {
        "today"
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let base_set: HashSet<Oid> = chunk_today(&rt, base)?.iter().map(|c| c.id).collect();
    for v in variants {
        let t = Instant::now();
        let chunks = chunk_today(&rt, v)?;
        let total: u64 = chunks.iter().map(|c| c.size).sum();
        let mut seen = HashSet::new();
        let mut new = 0u64;
        let (mut by_type, mut by_codec) = (BTreeMap::new(), BTreeMap::new());
        for c in &chunks {
            *by_type.entry(c.ctype.clone()).or_insert(0u64) += 1;
            *by_codec.entry(c.codec.clone()).or_insert(0u64) += 1;
            if !base_set.contains(&c.id) && seen.insert(c.id) {
                new += c.size;
            }
        }
        println!(
            "{}",
            json!({
                "mode": label, "base": file_name(base), "variant": file_name(v),
                "variant_bytes": total, "new_bytes": new,
                "reuse_pct": reuse_pct(new, total),
                "chunk_count": chunks.len(),
                "avg_chunk": if chunks.is_empty() { 0 } else { total / chunks.len() as u64 },
                "by_chunk_type": by_type, "by_codec_hint": by_codec,
                "ms": t.elapsed().as_millis() as u64,
            })
        );
    }
    Ok(())
}

// ───────────────────────── MOV/MP4 parsing ─────────────────────────

#[derive(Clone, Copy)]
struct Sample {
    track: usize,
    off: u64,
    size: u64,
}

struct Track {
    handler: String, // vide / soun / tmcd / ...
    fourcc: String,  // first stsd entry
    sync_count: usize,
}

struct Parsed {
    file_len: u64,
    tracks: Vec<Track>,
    samples: Vec<Sample>, // sorted by offset
    anomalies: Vec<String>,
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn be64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Split an in-memory buffer into child boxes: (fourcc, body).
fn boxes(buf: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 8 <= buf.len() {
        let sz = be32(buf, p).unwrap() as u64;
        let ty: [u8; 4] = buf[p + 4..p + 8].try_into().unwrap();
        let (hdr, total) = match sz {
            0 => (8, (buf.len() - p) as u64),
            1 => match be64(buf, p + 8) {
                Some(s) => (16, s),
                None => break,
            },
            s => (8, s),
        };
        if total < hdr as u64 || p as u64 + total > buf.len() as u64 {
            break;
        }
        out.push((ty, &buf[p + hdr..p + total as usize]));
        p += total as usize;
    }
    out
}

fn child<'a>(buf: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(buf)
        .into_iter()
        .find(|(t, _)| t == ty)
        .map(|(_, b)| b)
}

fn s(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

/// Walk top-level atoms with seeks and return the moov body (read into memory).
fn read_moov(f: &mut File, file_len: u64) -> Result<Vec<u8>> {
    let mut pos = 0u64;
    while pos + 8 <= file_len {
        f.seek(SeekFrom::Start(pos))?;
        let mut h = [0u8; 16];
        let n = f.read(&mut h)?;
        if n < 8 {
            break;
        }
        let sz = u32::from_be_bytes(h[0..4].try_into().unwrap()) as u64;
        let ty = &h[4..8];
        let (hdr, total) = match sz {
            0 => (8, file_len - pos),
            1 => {
                if n < 16 {
                    break;
                }
                (16, u64::from_be_bytes(h[8..16].try_into().unwrap()))
            }
            s => (8, s),
        };
        if total < hdr as u64 {
            bail!("bad atom size at {pos}");
        }
        if ty == b"moov" {
            f.seek(SeekFrom::Start(pos + hdr as u64))?;
            let mut v = vec![0u8; (total - hdr as u64) as usize];
            f.read_exact(&mut v)?;
            return Ok(v);
        }
        pos += total;
    }
    bail!("no moov found")
}

fn parse(path: &str) -> Result<Parsed> {
    let mut f = File::open(path).with_context(|| path.to_string())?;
    let file_len = f.metadata()?.len();
    let moov = read_moov(&mut f, file_len)?;
    let mut p = Parsed {
        file_len,
        tracks: vec![],
        samples: vec![],
        anomalies: vec![],
    };

    for (ty, trak) in boxes(&moov) {
        if &ty != b"trak" {
            continue;
        }
        let ti = p.tracks.len();
        let Some(mdia) = child(trak, b"mdia") else {
            continue;
        };
        let handler = child(mdia, b"hdlr")
            .and_then(|h| h.get(8..12))
            .map(s)
            .unwrap_or_default();
        let Some(stbl) = child(mdia, b"minf").and_then(|m| child(m, b"stbl")) else {
            p.anomalies.push(format!("track {ti}: no stbl"));
            continue;
        };
        let fourcc = child(stbl, b"stsd")
            .and_then(|d| d.get(12..16))
            .map(s)
            .unwrap_or_default();

        // stsz
        let (mut sizes, mut const_size, mut count) = (Vec::new(), 0u32, 0usize);
        if let Some(z) = child(stbl, b"stsz") {
            const_size = be32(z, 4).unwrap_or(0);
            count = be32(z, 8).unwrap_or(0) as usize;
            if const_size == 0 {
                sizes = (0..count).filter_map(|i| be32(z, 12 + 4 * i)).collect();
                if sizes.len() != count {
                    p.anomalies.push(format!("track {ti}: truncated stsz"));
                }
            }
        }
        // chunk offsets
        let offsets: Vec<u64> = if let Some(c) = child(stbl, b"stco") {
            let n = be32(c, 4).unwrap_or(0) as usize;
            (0..n)
                .filter_map(|i| be32(c, 8 + 4 * i).map(u64::from))
                .collect()
        } else if let Some(c) = child(stbl, b"co64") {
            let n = be32(c, 4).unwrap_or(0) as usize;
            (0..n).filter_map(|i| be64(c, 8 + 8 * i)).collect()
        } else {
            p.anomalies.push(format!("track {ti}: no stco/co64"));
            continue;
        };
        // stsc runs: (first_chunk, samples_per_chunk)
        let runs: Vec<(u32, u32)> = child(stbl, b"stsc")
            .map(|c| {
                let n = be32(c, 4).unwrap_or(0) as usize;
                (0..n)
                    .filter_map(|i| Some((be32(c, 8 + 12 * i)?, be32(c, 12 + 12 * i)?)))
                    .collect()
            })
            .unwrap_or_default();
        let sync_count = child(stbl, b"stss").and_then(|c| be32(c, 4)).unwrap_or(0) as usize;

        // expand to absolute (offset, size) per sample
        let mut si = 0usize;
        let mut ri = 0usize;
        for (ci, &coff) in offsets.iter().enumerate() {
            let chunk_no = ci as u32 + 1;
            while ri + 1 < runs.len() && runs[ri + 1].0 <= chunk_no {
                ri += 1;
            }
            let spc = runs.get(ri).map(|r| r.1).unwrap_or(0);
            let mut off = coff;
            for _ in 0..spc {
                if si >= count {
                    break;
                }
                let sz = if const_size != 0 {
                    const_size
                } else {
                    sizes[si]
                } as u64;
                p.samples.push(Sample {
                    track: ti,
                    off,
                    size: sz,
                });
                off += sz;
                si += 1;
            }
        }
        if si != count {
            p.anomalies.push(format!(
                "track {ti}: expanded {si} samples, stsz says {count}"
            ));
        }
        p.tracks.push(Track {
            handler,
            fourcc,
            sync_count,
        });
    }
    p.samples.sort_by_key(|x| x.off);
    Ok(p)
}

/// Drop out-of-bounds and overlapping samples (counted as anomalies) and
/// return the sorted, disjoint, in-bounds remainder.
fn clean_samples(p: &Parsed) -> (Vec<Sample>, usize, usize) {
    let (mut oob, mut overlap) = (0, 0);
    let mut cursor = 0u64;
    let mut out = Vec::with_capacity(p.samples.len());
    for s in &p.samples {
        if s.off + s.size > p.file_len {
            oob += 1;
        } else if s.off < cursor {
            overlap += 1;
        } else {
            cursor = s.off + s.size;
            out.push(*s);
        }
    }
    (out, oob, overlap)
}

// ───────────────────────────── ideal ─────────────────────────────

/// blake3 of every sample, in file order, via one buffered pass.
fn hash_samples(path: &str, samples: &[Sample]) -> Result<Vec<blake3::Hash>> {
    let mut r = BufReader::with_capacity(MIB as usize, File::open(path)?);
    let mut pos = 0u64;
    let mut buf = Vec::new();
    let mut out = Vec::with_capacity(samples.len());
    for s in samples {
        r.seek_relative(s.off as i64 - pos as i64)?;
        buf.resize(s.size as usize, 0);
        r.read_exact(&mut buf)?;
        pos = s.off + s.size;
        out.push(blake3::hash(&buf));
    }
    Ok(out)
}

fn run_ideal(base: &str, variants: &[String]) -> Result<()> {
    let bp = parse(base)?;
    let (bs, _, _) = clean_samples(&bp);
    let base_set: HashSet<blake3::Hash> = hash_samples(base, &bs)?.into_iter().collect();
    for v in variants {
        let vp = parse(v)?;
        let (vs, oob, overlap) = clean_samples(&vp);
        let hashes = hash_samples(v, &vs)?;
        let mut seen = HashSet::new();
        // per handler type: (count, bytes, new_bytes)
        let mut per: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
        let (mut sample_bytes, mut new_sample_bytes) = (0u64, 0u64);
        for (s, h) in vs.iter().zip(&hashes) {
            let e = per.entry(vp.tracks[s.track].handler.clone()).or_default();
            e.0 += 1;
            e.1 += s.size;
            sample_bytes += s.size;
            if !base_set.contains(h) && seen.insert(*h) {
                e.2 += s.size;
                new_sample_bytes += s.size;
            }
        }
        let non_sample = vp.file_len - sample_bytes;
        let new = new_sample_bytes + non_sample;
        let per_json: BTreeMap<_, _> = per
            .iter()
            .map(|(k, (c, b, n))| (k.clone(), json!({"samples": c, "bytes": b, "new_bytes": n})))
            .collect();
        let tracks: Vec<_> = vp
            .tracks
            .iter()
            .map(|t| json!({"handler": t.handler, "stsd": t.fourcc, "sync": t.sync_count}))
            .collect();
        println!(
            "{}",
            json!({
                "mode": "ideal", "base": file_name(base), "variant": file_name(v),
                "variant_bytes": vp.file_len, "non_sample_bytes": non_sample,
                "new_bytes": new, "reuse_pct": reuse_pct(new, vp.file_len),
                "per_track_type": per_json, "tracks": tracks,
                "oob_samples": oob, "overlap_samples": overlap,
                "anomalies": vp.anomalies,
            })
        );
    }
    Ok(())
}

// ───────────────────────────── frame ─────────────────────────────

struct Plan {
    off: u64,
    len: u64,
    class: &'static str, // video / grouped / gap
}

/// Cut [off, off+len) into pieces of at most `max` bytes.
fn push_split(plan: &mut Vec<Plan>, off: u64, len: u64, max: u64, class: &'static str) {
    let mut o = off;
    while o < off + len {
        let l = max.min(off + len - o);
        plan.push(Plan {
            off: o,
            len: l,
            class,
        });
        o += l;
    }
}

/// Build the contiguous chunk plan covering [0, file_len).
fn plan_frame(p: &Parsed, samples: &[Sample]) -> Vec<Plan> {
    let mut plan = Vec::new();
    let mut cursor = 0u64;
    // open group of adjacent same-track non-video samples: (track, start, end)
    let mut grp: Option<(usize, u64, u64)> = None;
    let flush = |plan: &mut Vec<Plan>, g: &mut Option<(usize, u64, u64)>| {
        if let Some((_, a, b)) = g.take() {
            plan.push(Plan {
                off: a,
                len: b - a,
                class: "grouped",
            });
        }
    };
    for s in samples {
        if s.off > cursor {
            flush(&mut plan, &mut grp);
            push_split(&mut plan, cursor, s.off - cursor, GAP_PIECE, "gap");
        }
        if p.tracks[s.track].handler == "vide" {
            flush(&mut plan, &mut grp);
            push_split(&mut plan, s.off, s.size, VIDEO_PIECE, "video");
        } else if s.size >= GROUP_MAX {
            flush(&mut plan, &mut grp);
            push_split(&mut plan, s.off, s.size, GROUP_MAX, "grouped");
        } else {
            match grp {
                Some((t, a, e)) if t == s.track && e == s.off && e + s.size - a <= GROUP_MAX => {
                    grp.as_mut().unwrap().2 = s.off + s.size;
                }
                _ => {
                    flush(&mut plan, &mut grp);
                    grp = Some((s.track, s.off, s.off + s.size));
                }
            }
        }
        cursor = s.off + s.size;
    }
    flush(&mut plan, &mut grp);
    if cursor < p.file_len {
        push_split(&mut plan, cursor, p.file_len - cursor, GAP_PIECE, "gap");
    }
    // invariant: contiguous and covers the file
    let mut exp = 0u64;
    for c in &plan {
        assert_eq!(c.off, exp, "frame plan not contiguous");
        exp += c.len;
    }
    assert_eq!(exp, p.file_len, "frame plan does not sum to file size");
    plan
}

/// Hash each planned chunk with the production Oid hash, streaming the file.
fn hash_plan(path: &str, plan: &[Plan]) -> Result<Vec<Oid>> {
    let mut r = BufReader::with_capacity(MIB as usize, File::open(path)?);
    let mut buf = Vec::new();
    let mut out = Vec::with_capacity(plan.len());
    for c in plan {
        buf.resize(c.len as usize, 0);
        r.read_exact(&mut buf)?; // plan is contiguous: pure sequential read
        out.push(Oid::hash(&buf));
    }
    Ok(out)
}

fn run_frame(base: &str, variants: &[String]) -> Result<()> {
    let bp = parse(base)?;
    let (bs, _, _) = clean_samples(&bp);
    let base_set: HashSet<Oid> = hash_plan(base, &plan_frame(&bp, &bs))?
        .into_iter()
        .collect();
    for v in variants {
        let t = Instant::now();
        let vp = parse(v)?;
        let (vs, _, _) = clean_samples(&vp);
        let plan = plan_frame(&vp, &vs);
        let ids = hash_plan(v, &plan)?;
        let mut seen = HashSet::new();
        let mut new = 0u64;
        let mut by_class: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for (c, id) in plan.iter().zip(&ids) {
            let e = by_class.entry(c.class).or_default();
            e.0 += 1;
            e.1 += c.len;
            if !base_set.contains(id) && seen.insert(*id) {
                new += c.len;
            }
        }
        let by_class_json: BTreeMap<_, _> = by_class
            .iter()
            .map(|(k, (n, b))| (k.to_string(), json!({"chunks": n, "bytes": b})))
            .collect();
        println!(
            "{}",
            json!({
                "mode": "frame", "base": file_name(base), "variant": file_name(v),
                "variant_bytes": vp.file_len, "new_bytes": new,
                "reuse_pct": reuse_pct(new, vp.file_len),
                "chunk_count": plan.len(),
                "avg_chunk": vp.file_len / plan.len().max(1) as u64,
                "by_class": by_class_json, "chunks_sum_eq_file": true,
                "ms": t.elapsed().as_millis() as u64,
            })
        );
    }
    Ok(())
}

/// Peak working set (Windows) in MiB; same PowerShell shim as dedup_report.
fn peak_rss_mb() -> Option<f64> {
    let pid = std::process::id();
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
        ])
        .output()
        .ok()?;
    let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    Some(bytes as f64 / MIB as f64)
}
