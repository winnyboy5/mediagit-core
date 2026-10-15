// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Streaming, frame-aligned MOV/MP4 chunker.
//!
//! The sample tables in `moov` (`stsz`/`stco`/`co64`/`stsc`) say exactly where
//! every media sample lives. Large video samples (>= [`FRAME_MIN`]) become one
//! chunk each, so an edited timeline reuses every untouched frame regardless
//! of byte shifts. Everything else (atom headers, `moov`, audio, small video
//! samples, gaps) is content-defined (FastCDC) *between* those frames, so PCM
//! and interleaved AAC/H.264 stay shift-resistant without a chunk explosion.
//!
//! Work is split in two phases so a fallback is always clean:
//! [`Mp4Plan::build`] reads only the atom headers and `moov` and validates
//! everything; nothing is emitted until it succeeds. [`Mp4Plan::run`] then
//! streams the file once, in offset order, holding one chunk at a time.

use super::formats::mp4_fourcc_to_codec_hint;
use super::{ChunkType, CodecHint, ContentChunk, codec_detect_enabled};
use crate::Oid;
use anyhow::{Result, anyhow, bail};
use std::io::{Cursor, Read, Seek, SeekFrom};

/// Video samples at least this big get their own chunk.
const FRAME_MIN: u64 = 256 * 1024;
/// Hard ceiling on any emitted chunk (RSS budget: in-flight chunks x this).
const MAX_CHUNK: u64 = 4 * 1024 * 1024;
/// `moov` is held in memory; refuse anything bigger (caller falls back).
const MOOV_CAP: u64 = 256 * 1024 * 1024;
/// Guard against pathological files made of millions of tiny atoms.
const MAX_TOP_ATOMS: usize = 1 << 20;
/// Hard caps on table expansion work (malicious `stsz`/`stsc`/`stco` combos).
const MAX_RUNS: usize = 16 * 1024 * 1024;
const MAX_SAMPLE_ITERS: u64 = 64 * 1024 * 1024;
const CDC_MIN: usize = 256 * 1024;
const CDC_AVG: usize = 1024 * 1024;
const CDC_MAX: usize = MAX_CHUNK as usize;

/// `MEDIAGIT_VIDEO_CHUNKER=cdc` restores the old frame-blind behaviour;
/// anything else (including unset) uses the frame-aligned walker.
pub(crate) fn frame_chunker_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        frame_chunker_from_env(std::env::var("MEDIAGIT_VIDEO_CHUNKER").ok().as_deref())
    })
}

/// Split out so tests can check parsing without the process-wide `OnceLock`.
fn frame_chunker_from_env(var: Option<&str>) -> bool {
    var.map(|v| v != "cdc" && v != "0").unwrap_or(true)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Meta,
    Other,
    Video(CodecHint),
    Audio(CodecHint),
}

#[derive(Clone, Copy, Debug)]
struct Run {
    off: u64,
    len: u64,
    role: Role,
    standalone: bool,
}

#[derive(Clone, Copy, Debug)]
enum Seg {
    Frame { off: u64, len: u64, hint: CodecHint },
    Filler { off: u64, len: u64 },
}

impl Seg {
    fn range(&self) -> (u64, u64) {
        match *self {
            Seg::Frame { off, len, .. } | Seg::Filler { off, len } => (off, len),
        }
    }
}

/// A validated chunking plan covering `[0, file_len)` exactly.
pub(crate) struct Mp4Plan {
    file_len: u64,
    segs: Vec<Seg>,
    /// Sorted, disjoint sample runs; used to tag CDC chunks.
    roles: Vec<Run>,
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        b.get(o..o.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn be64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        b.get(o..o.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// Child boxes of an in-memory buffer: (fourcc, body). `None` if any box is
/// malformed (size below header, or past the end of the buffer).
fn boxes(buf: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 8 <= buf.len() {
        let sz = be32(buf, p).unwrap_or(0) as u64;
        let ty: [u8; 4] = buf[p + 4..p + 8].try_into().unwrap_or([0; 4]);
        let (hdr, total) = match sz {
            0 => (8u64, (buf.len() - p) as u64),
            1 => (16, be64(buf, p + 8)?),
            s => (8, s),
        };
        if total < hdr || (p as u64).checked_add(total)? > buf.len() as u64 {
            return None;
        }
        out.push((ty, &buf[p + hdr as usize..p + total as usize]));
        p += total as usize;
    }
    Some(out)
}

fn child<'a>(buf: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(buf)?
        .into_iter()
        .find(|(t, _)| t == ty)
        .map(|(_, b)| b)
}

struct Atom {
    kind: [u8; 4],
    off: u64,
    len: u64,
    hdr: u64,
}

/// Walk top-level atoms with seeks, validating every size.
fn top_level<R: Read + Seek>(r: &mut R, file_len: u64) -> Result<Vec<Atom>> {
    let mut out = Vec::new();
    let mut pos = 0u64;
    while pos + 8 <= file_len {
        if out.len() >= MAX_TOP_ATOMS {
            bail!("too many top-level atoms");
        }
        r.seek(SeekFrom::Start(pos))?;
        let want = (file_len - pos).min(16) as usize;
        let mut h = [0u8; 16];
        r.read_exact(&mut h[..want])?;
        let size32 = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as u64;
        let kind = [h[4], h[5], h[6], h[7]];
        let (hdr, len) = match size32 {
            0 => (8, file_len - pos),
            1 => {
                if want < 16 {
                    bail!("truncated extended atom header at {pos}");
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&h[8..16]);
                (16, u64::from_be_bytes(b))
            }
            s => (8, s),
        };
        if len < hdr {
            bail!("atom size {len} smaller than header at {pos}");
        }
        let end = pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("atom size overflow at {pos}"))?;
        if end > file_len {
            bail!("atom at {pos} extends past EOF");
        }
        if &kind == b"moof" {
            bail!("fragmented MP4 (moof)");
        }
        out.push(Atom {
            kind,
            off: pos,
            len,
            hdr,
        });
        pos = end;
    }
    Ok(out)
}

fn is_pcm(fourcc: &[u8; 4]) -> bool {
    matches!(
        fourcc,
        b"sowt" | b"twos" | b"lpcm" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"raw "
    )
}

/// Running totals across all tracks. Valid samples cannot overlap, so their
/// total can never exceed the file; anything bigger is a crafted table and is
/// rejected before it can expand into billions of runs.
struct Budget {
    file_len: u64,
    bytes: u64,
    iters: u64,
}

impl Budget {
    fn iterate(&mut self, n: u64) -> Result<()> {
        self.iters = self.iters.saturating_add(n);
        if self.iters > MAX_SAMPLE_ITERS {
            bail!("sample tables too large");
        }
        Ok(())
    }
}

/// Append a run, merging contiguous small runs of the same role.
fn push_run(runs: &mut Vec<Run>, budget: &mut Budget, run: Run) -> Result<()> {
    let file_len = budget.file_len;
    if run.len == 0 {
        return Ok(());
    }
    budget.bytes = budget.bytes.saturating_add(run.len);
    if budget.bytes > file_len {
        bail!("sample bytes exceed file size");
    }
    let end = run
        .off
        .checked_add(run.len)
        .ok_or_else(|| anyhow!("sample range overflow"))?;
    if end > file_len {
        bail!("sample range {}..{} past EOF {}", run.off, end, file_len);
    }
    if !run.standalone
        && let Some(last) = runs.last_mut()
        && !last.standalone
        && last.role == run.role
        && last.off + last.len == run.off
    {
        last.len += run.len;
        return Ok(());
    }
    if runs.len() >= MAX_RUNS {
        bail!("too many sample runs");
    }
    runs.push(run);
    Ok(())
}

/// Turn one `trak` into sample runs. Video tracks get per-sample runs; every
/// other track one run per stco chunk (PCM has ~50k samples/s).
fn add_track(trak: &[u8], budget: &mut Budget, detect: bool, runs: &mut Vec<Run>) -> Result<()> {
    let Some(mdia) = child(trak, b"mdia") else {
        return Ok(());
    };
    let handler: [u8; 4] = child(mdia, b"hdlr")
        .and_then(|h| h.get(8..12))
        .and_then(|h| h.try_into().ok())
        .unwrap_or([0; 4]);
    let stbl = child(mdia, b"minf")
        .and_then(|m| child(m, b"stbl"))
        .ok_or_else(|| anyhow!("track without stbl"))?;
    // Only the first stsd entry sets the hint (multi-entry tracks are rare).
    let fourcc: [u8; 4] = child(stbl, b"stsd")
        .and_then(|d| d.get(12..16))
        .and_then(|d| d.try_into().ok())
        .unwrap_or([0; 4]);
    let hint = if detect {
        match mp4_fourcc_to_codec_hint(&fourcc) {
            CodecHint::Unknown if &handler == b"soun" && is_pcm(&fourcc) => CodecHint::PCM,
            h => h,
        }
    } else {
        CodecHint::Unknown
    };
    let is_video = &handler == b"vide";
    let role = match &handler {
        b"vide" => Role::Video(hint),
        b"soun" => Role::Audio(hint),
        _ => Role::Other,
    };

    let z = child(stbl, b"stsz").ok_or_else(|| anyhow!("no stsz"))?;
    let const_size = be32(z, 4).ok_or_else(|| anyhow!("short stsz"))? as u64;
    let count = be32(z, 8).ok_or_else(|| anyhow!("short stsz"))? as u64;
    if const_size == 0 && 12 + 4 * count > z.len() as u64 {
        bail!("truncated stsz");
    }
    let sample_size = |i: u64| -> u64 {
        if const_size != 0 {
            const_size
        } else {
            be32(z, 12 + 4 * i as usize).unwrap_or(0) as u64
        }
    };

    let (co, wide) = if let Some(c) = child(stbl, b"stco") {
        (c, false)
    } else if let Some(c) = child(stbl, b"co64") {
        (c, true)
    } else {
        bail!("no stco/co64");
    };
    let n_chunks = be32(co, 4).ok_or_else(|| anyhow!("short stco"))? as u64;
    let w = if wide { 8 } else { 4 };
    if 8 + w * n_chunks > co.len() as u64 {
        bail!("truncated stco");
    }
    let chunk_off = |i: u64| -> u64 {
        let o = 8 + (w * i) as usize;
        if wide {
            be64(co, o).unwrap_or(u64::MAX)
        } else {
            be32(co, o).unwrap_or(u32::MAX) as u64
        }
    };

    let sc = child(stbl, b"stsc").ok_or_else(|| anyhow!("no stsc"))?;
    let n_runs = be32(sc, 4).ok_or_else(|| anyhow!("short stsc"))? as u64;
    if 8 + 12 * n_runs > sc.len() as u64 || (n_runs == 0 && count > 0) {
        bail!("bad stsc");
    }
    let stsc_first = |i: u64| be32(sc, 8 + 12 * i as usize).unwrap_or(u32::MAX) as u64;
    let stsc_spc = |i: u64| be32(sc, 12 + 12 * i as usize).unwrap_or(0) as u64;

    let mut si = 0u64;
    let mut ri = 0u64;
    for ci in 0..n_chunks {
        if si >= count {
            break;
        }
        while ri + 1 < n_runs && stsc_first(ri + 1) <= ci + 1 {
            ri += 1;
        }
        let spc = stsc_spc(ri).min(count - si);
        let off0 = chunk_off(ci);
        if is_video {
            budget.iterate(spc)?;
            let mut off = off0;
            for _ in 0..spc {
                let sz = sample_size(si);
                let run = Run {
                    off,
                    len: sz,
                    role,
                    standalone: sz >= FRAME_MIN,
                };
                push_run(runs, budget, run)?;
                off = off
                    .checked_add(sz)
                    .ok_or_else(|| anyhow!("offset overflow"))?;
                si += 1;
            }
        } else {
            let total = if const_size != 0 {
                const_size * spc
            } else {
                budget.iterate(spc)?;
                (si..si + spc).map(sample_size).sum()
            };
            si += spc;
            push_run(
                runs,
                budget,
                Run {
                    off: off0,
                    len: total,
                    role,
                    standalone: false,
                },
            )?;
        }
    }
    if si != count {
        bail!("track expanded {si} samples, stsz says {count}");
    }
    Ok(())
}

impl Mp4Plan {
    /// Parse and validate. Reads only atom headers and `moov`.
    pub(crate) fn build<R: Read + Seek>(r: &mut R) -> Result<Self> {
        let file_len = r.seek(SeekFrom::End(0))?;
        let atoms = top_level(r, file_len)?;
        let moov = atoms
            .iter()
            .find(|a| &a.kind == b"moov")
            .ok_or_else(|| anyhow!("no moov"))?;
        let body_len = moov.len - moov.hdr;
        if body_len > MOOV_CAP {
            bail!("moov of {body_len} bytes exceeds cap");
        }
        r.seek(SeekFrom::Start(moov.off + moov.hdr))?;
        let mut body = vec![0u8; body_len as usize];
        r.read_exact(&mut body)?;

        let detect = codec_detect_enabled();
        let mut runs = Vec::new();
        let mut budget = Budget {
            file_len,
            bytes: 0,
            iters: 0,
        };
        for (ty, trak) in boxes(&body).ok_or_else(|| anyhow!("malformed moov"))? {
            if &ty == b"trak" {
                if boxes(trak).is_none() {
                    bail!("malformed trak");
                }
                add_track(trak, &mut budget, detect, &mut runs)?;
            }
        }
        runs.sort_unstable_by_key(|r| r.off);
        for w in runs.windows(2) {
            if w[0].off + w[0].len > w[1].off {
                bail!("overlapping samples at {}", w[1].off);
            }
        }

        // Hard cuts at every moov/mdat boundary.
        let mut cuts: Vec<u64> = atoms
            .iter()
            .filter(|a| &a.kind == b"moov" || &a.kind == b"mdat")
            .flat_map(|a| [a.off, a.off + a.len])
            .collect();
        cuts.sort_unstable();
        cuts.dedup();

        let mut segs = Vec::new();
        let mut cursor = 0u64;
        for run in runs.iter().filter(|r| r.standalone) {
            push_filler(&mut segs, &cuts, cursor, run.off);
            let Role::Video(hint) = run.role else {
                unreachable!("only video runs are standalone")
            };
            segs.push(Seg::Frame {
                off: run.off,
                len: run.len,
                hint,
            });
            cursor = run.off + run.len;
        }
        push_filler(&mut segs, &cuts, cursor, file_len);

        // Self-check: contiguous, covers the file exactly.
        let mut at = 0u64;
        for s in &segs {
            let (off, len) = s.range();
            if off != at || len == 0 {
                bail!("plan not contiguous at {at}");
            }
            at += len;
        }
        if at != file_len {
            bail!("plan covers {at} of {file_len} bytes");
        }
        Ok(Self {
            file_len,
            segs,
            roles: runs,
        })
    }

    /// Number of standalone frame segments (one chunk each, unless > 4 MiB).
    #[cfg(test)]
    fn frame_count(&self) -> usize {
        self.segs
            .iter()
            .filter(|s| matches!(s, Seg::Frame { .. }))
            .count()
    }

    /// Tag for a CDC chunk: the role contributing the most bytes wins.
    fn classify(&self, start: u64, end: u64) -> (ChunkType, CodecHint) {
        let mut acc: Vec<(Role, u64)> = Vec::new();
        let mut add = |role: Role, n: u64| {
            if n == 0 {
                return;
            }
            match acc.iter_mut().find(|(r, _)| *r == role) {
                Some((_, b)) => *b += n,
                None => acc.push((role, n)),
            }
        };
        let mut covered = 0u64;
        let mut i = self.roles.partition_point(|r| r.off + r.len <= start);
        while let Some(r) = self.roles.get(i) {
            if r.off >= end {
                break;
            }
            let n = (r.off + r.len).min(end) - r.off.max(start);
            covered += n;
            add(r.role, n);
            i += 1;
        }
        add(Role::Meta, (end - start) - covered);
        let best = acc
            .iter()
            .fold(None::<(Role, u64)>, |b, &(r, n)| match b {
                Some((_, bn)) if bn >= n => b,
                _ => Some((r, n)),
            })
            .map(|(r, _)| r)
            .unwrap_or(Role::Meta);
        match best {
            Role::Meta => (ChunkType::Metadata, CodecHint::Unknown),
            Role::Other => (ChunkType::Generic, CodecHint::Unknown),
            Role::Video(h) => (ChunkType::VideoStream, h),
            Role::Audio(h) => (ChunkType::AudioStream, h),
        }
    }

    /// Stream the file once, in offset order, sending each chunk to `emit`.
    /// Errors if the emitted chunks are not an exact contiguous cover.
    pub(crate) fn run<R: Read + Seek>(
        &self,
        r: &mut R,
        seed: u64,
        emit: &mut dyn FnMut(ContentChunk) -> Result<()>,
    ) -> Result<()> {
        let mut cursor = 0u64;
        let mut push = |data: Vec<u8>, ct: ChunkType, hint: CodecHint| -> Result<()> {
            let size = data.len();
            if size == 0 || size as u64 > MAX_CHUNK {
                bail!("bad chunk size {size}");
            }
            let chunk = ContentChunk {
                id: Oid::hash(&data),
                data,
                offset: cursor,
                size,
                chunk_type: ct,
                perceptual_hash: None,
                codec_hint: hint,
            };
            cursor += size as u64;
            emit(chunk)
        };

        for seg in &self.segs {
            let (off, len) = seg.range();
            r.seek(SeekFrom::Start(off))?;
            match *seg {
                Seg::Frame { hint, .. } => {
                    let mut done = 0u64;
                    while done < len {
                        let n = (len - done).min(MAX_CHUNK) as usize;
                        let mut buf = vec![0u8; n];
                        r.read_exact(&mut buf)?;
                        push(buf, ChunkType::VideoStream, hint)?;
                        done += n as u64;
                    }
                }
                Seg::Filler { .. } if len <= CDC_MIN as u64 => {
                    // Short gap: one chunk, no 4 MiB StreamCDC buffer.
                    let mut buf = vec![0u8; len as usize];
                    r.read_exact(&mut buf)?;
                    let (ct, hint) = self.classify(off, off + len);
                    push(buf, ct, hint)?;
                }
                Seg::Filler { .. } => {
                    let cdc = fastcdc::v2020::StreamCDC::with_level_and_seed(
                        (&mut *r).take(len),
                        CDC_MIN,
                        CDC_AVG,
                        CDC_MAX,
                        fastcdc::v2020::Normalization::Level1,
                        seed,
                    );
                    for e in cdc {
                        let e = e.map_err(|e| anyhow!("FastCDC streaming error: {e}"))?;
                        let abs = off + e.offset;
                        let (ct, hint) = self.classify(abs, abs + e.length as u64);
                        push(e.data, ct, hint)?;
                    }
                }
            }
        }
        if cursor != self.file_len || r.seek(SeekFrom::End(0))? != self.file_len {
            bail!(
                "file changed while chunking (expected {} bytes)",
                self.file_len
            );
        }
        Ok(())
    }
}

/// Push `[from, to)` as filler segments, split at every hard-cut boundary.
fn push_filler(segs: &mut Vec<Seg>, cuts: &[u64], from: u64, to: u64) {
    let mut a = from;
    let mut i = cuts.partition_point(|&c| c <= from);
    while a < to {
        let b = match cuts.get(i) {
            Some(&c) if c < to => c,
            _ => to,
        };
        segs.push(Seg::Filler { off: a, len: b - a });
        a = b;
        i += 1;
    }
}

/// In-memory entry point for `chunk_mp4`: same walker over a `Cursor`.
pub(crate) fn chunk_slice(data: &[u8], seed: u64) -> Result<Vec<ContentChunk>> {
    let mut cur = Cursor::new(data);
    let plan = Mp4Plan::build(&mut cur)?;
    let mut out = Vec::new();
    plan.run(&mut cur, seed, &mut |c| {
        out.push(c);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chunking::{ChunkStrategy, ContentChunker};
    use proptest::prelude::*;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((8 + body.len()) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    fn payload(seed: u64, n: usize) -> Vec<u8> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    struct Spec<'a> {
        handler: &'a [u8; 4],
        fourcc: &'a [u8; 4],
        const_size: u32,
        sizes: Vec<u32>,
        count: u32,
        offsets: Vec<u32>,
        spc: u32,
    }

    fn trak(s: &Spec) -> Vec<u8> {
        let mut hdlr = vec![0u8; 8];
        hdlr.extend_from_slice(s.handler);
        hdlr.extend_from_slice(&[0u8; 13]);
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd.extend_from_slice(&atom(s.fourcc, &[]));
        let mut stsz = vec![0u8; 4];
        stsz.extend_from_slice(&s.const_size.to_be_bytes());
        stsz.extend_from_slice(&s.count.to_be_bytes());
        for z in &s.sizes {
            stsz.extend_from_slice(&z.to_be_bytes());
        }
        let mut stsc = vec![0, 0, 0, 0, 0, 0, 0, 1];
        for v in [1u32, s.spc, 1] {
            stsc.extend_from_slice(&v.to_be_bytes());
        }
        let mut stco = vec![0, 0, 0, 0];
        stco.extend_from_slice(&(s.offsets.len() as u32).to_be_bytes());
        for o in &s.offsets {
            stco.extend_from_slice(&o.to_be_bytes());
        }
        let mut stbl = atom(b"stsd", &stsd);
        stbl.extend(atom(b"stsz", &stsz));
        stbl.extend(atom(b"stsc", &stsc));
        stbl.extend(atom(b"stco", &stco));
        let minf = atom(b"minf", &atom(b"stbl", &stbl));
        let mut mdia = atom(b"hdlr", &hdlr);
        mdia.extend(minf);
        atom(b"trak", &atom(b"mdia", &mdia))
    }

    /// ftyp | mdat(interleaved video sample + PCM-like audio chunk) | moov.
    /// Returns (file, [(offset, size)] of video samples).
    pub(crate) fn make_mp4(video: &[u32], audio_spc: u32) -> (Vec<u8>, Vec<(u64, u32)>) {
        let ftyp = atom(b"ftyp", b"isom\x00\x00\x00\x01isom");
        let mut mdat_body = Vec::new();
        let (mut voff, mut aoff, mut vsamples) = (Vec::new(), Vec::new(), Vec::new());
        let base = ftyp.len() as u32 + 8;
        for (i, &sz) in video.iter().enumerate() {
            voff.push(base + mdat_body.len() as u32);
            vsamples.push(((base + mdat_body.len() as u32) as u64, sz));
            mdat_body.extend(payload(i as u64, sz as usize));
            aoff.push(base + mdat_body.len() as u32);
            mdat_body.extend(payload(1000 + i as u64, audio_spc as usize * 2));
        }
        let n = video.len() as u32;
        let v = Spec {
            handler: b"vide",
            fourcc: b"avc1",
            const_size: 0,
            sizes: video.to_vec(),
            count: n,
            offsets: voff,
            spc: 1,
        };
        let a = Spec {
            handler: b"soun",
            fourcc: b"sowt",
            const_size: 2,
            sizes: vec![],
            count: n * audio_spc,
            offsets: aoff,
            spc: audio_spc,
        };
        let mut moov_body = trak(&v);
        moov_body.extend(trak(&a));
        let mut file = ftyp;
        file.extend(atom(b"mdat", &mdat_body));
        file.extend(atom(b"moov", &moov_body));
        (file, vsamples)
    }

    fn check_cover(chunks: &[ContentChunk], data: &[u8]) {
        let mut at = 0u64;
        let mut cat = Vec::with_capacity(data.len());
        for c in chunks {
            assert_eq!(c.offset, at, "chunks must be contiguous");
            assert_eq!(c.size, c.data.len());
            assert!(c.size as u64 <= MAX_CHUNK);
            at += c.size as u64;
            cat.extend_from_slice(&c.data);
        }
        assert_eq!(cat, data, "concatenation must equal input");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]
        #[test]
        fn covers_input_exactly(
            video in prop::collection::vec(prop_oneof![0u32..3000, 200_000u32..700_000], 1..10),
            spc in 1u32..4000,
        ) {
            let (data, vsamples) = make_mp4(&video, spc);
            let chunks = chunk_slice(&data, 7).unwrap();
            check_cover(&chunks, &data);
            for (off, sz) in vsamples {
                if sz as u64 >= FRAME_MIN {
                    let c = chunks.iter().find(|c| c.offset == off).expect("frame starts a chunk");
                    assert_eq!(c.size, sz as usize);
                    assert_eq!(c.chunk_type, ChunkType::VideoStream);
                    assert_eq!(c.codec_hint, CodecHint::H264);
                }
            }
        }
    }

    #[test]
    fn oversized_frame_is_split_at_4mib() {
        let (data, _) = make_mp4(&[9 * 1024 * 1024, 100], 10);
        let chunks = chunk_slice(&data, 7).unwrap();
        check_cover(&chunks, &data);
        assert!(chunks.iter().any(|c| c.size as u64 == MAX_CHUNK));
    }

    #[test]
    fn audio_chunks_tagged_audio_pcm() {
        let (data, _) = make_mp4(&[300_000, 300_000], 3000);
        let plan = Mp4Plan::build(&mut Cursor::new(&data)).unwrap();
        assert_eq!(plan.frame_count(), 2);
        let (ct, hint) = plan.classify(300_000 + 28, 300_000 + 28 + 6000);
        assert_eq!((ct, hint), (ChunkType::AudioStream, CodecHint::PCM));
    }

    fn patch_first_stco(data: &mut [u8], entry: usize, val: u32) {
        let at = data.windows(4).position(|w| w == b"stco").unwrap() + 12 + 4 * entry;
        data[at..at + 4].copy_from_slice(&val.to_be_bytes());
    }

    #[tokio::test]
    async fn malformed_inputs_fall_back_to_valid_cover() {
        let (good, _) = make_mp4(&[300_000, 300_000, 400_000], 100);
        assert!(Mp4Plan::build(&mut Cursor::new(&good)).is_ok());

        let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
        cases.push(("truncated", good[..good.len() - 50].to_vec()));
        let mut v = good.clone();
        v[20..24].copy_from_slice(&4u32.to_be_bytes()); // mdat size < header
        cases.push(("size<header", v));
        let mut v = good.clone();
        patch_first_stco(&mut v, 0, 0x7fff_ffff);
        cases.push(("offset past EOF", v));
        let mut v = good.clone();
        patch_first_stco(&mut v, 1, 28); // second frame overlaps the first
        cases.push(("overlap", v));
        let mut v = good.clone();
        v.extend(atom(b"moof", &[0u8; 8]));
        cases.push(("moof", v));

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        for (name, data) in cases {
            assert!(
                Mp4Plan::build(&mut Cursor::new(&data)).is_err(),
                "{name}: walker must reject"
            );
            let chunks = chunker.chunk(&data, "x.mp4").await.unwrap();
            let total: usize = chunks.iter().map(|c| c.size).sum();
            assert_eq!(total, data.len(), "{name}: fallback must cover the file");
            check_cover(&chunks, &data);
        }
    }

    fn file_with_moov(moov_body: &[u8]) -> Vec<u8> {
        let mut f = atom(b"ftyp", b"isom   isom");
        f.extend(atom(b"mdat", &payload(1, 100)));
        f.extend(atom(b"moov", moov_body));
        f
    }

    async fn assert_declined_and_covered(name: &str, data: &[u8]) {
        let t = std::time::Instant::now();
        assert!(
            Mp4Plan::build(&mut Cursor::new(data)).is_err(),
            "{name}: must reject"
        );
        assert!(t.elapsed().as_secs() < 2, "{name}: rejection must be fast");
        let chunks = ContentChunker::new(ChunkStrategy::MediaAware)
            .chunk(data, "x.mp4")
            .await
            .unwrap();
        check_cover(&chunks, data);
    }

    #[tokio::test]
    async fn nested_box_size_overflow_does_not_panic() {
        let mut moov = atom(b"free", &[0u8; 12]); // 20 bytes, so p = 20 for the next box
        moov.extend_from_slice(&1u32.to_be_bytes());
        moov.extend_from_slice(b"trak");
        moov.extend_from_slice(&(u64::MAX - 8).to_be_bytes());
        moov.extend_from_slice(&[0u8; 8]);
        assert_declined_and_covered("nested u64 overflow", &file_with_moov(&moov)).await;
    }

    #[tokio::test]
    async fn crafted_sample_tables_are_rejected_fast() {
        for (name, handler, count, offsets, spc) in [
            ("huge count video", b"vide", u32::MAX, vec![28u32], u32::MAX),
            ("huge count audio", b"soun", u32::MAX, vec![28u32], u32::MAX),
            (
                "duplicate stco offsets",
                b"vide",
                4000,
                vec![28u32; 4000],
                1,
            ),
        ] {
            let spec = Spec {
                handler,
                fourcc: b"avc1",
                const_size: 300_000,
                sizes: vec![],
                count,
                offsets,
                spc,
            };
            assert_declined_and_covered(name, &file_with_moov(&trak(&spec))).await;
        }
    }

    #[test]
    fn file_growing_after_build_is_an_error() {
        let (data, _) = make_mp4(&[300_000, 300_000], 100);
        let mut cur = Cursor::new(data);
        let plan = Mp4Plan::build(&mut cur).unwrap();
        cur.get_mut().extend_from_slice(&[1, 2, 3]);
        assert!(plan.run(&mut cur, 7, &mut |_| Ok(())).is_err());
    }

    #[test]
    fn knob_accepts_cdc_and_zero() {
        assert!(frame_chunker_from_env(None));
        assert!(frame_chunker_from_env(Some("frame")));
        assert!(!frame_chunker_from_env(Some("cdc")));
        assert!(!frame_chunker_from_env(Some("0")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn collect_file_chunks_blocking_uses_walker_and_falls_back() {
        let (good, _) = make_mp4(&[300_000, 10, 300_000], 100);
        let mut bad = good.clone();
        bad.truncate(bad.len() - 50);
        for (name, data, want_frames) in [("good.mp4", good, true), ("bad.mp4", bad, false)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            std::fs::write(&path, &data).unwrap();
            let chunker = ContentChunker::with_seed(ChunkStrategy::MediaAware, 7);
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            let h = tokio::task::spawn_blocking(move || {
                chunker.collect_file_chunks_blocking(&path, tx)
            });
            let mut chunks = Vec::new();
            while let Some(c) = rx.recv().await {
                chunks.push(c);
            }
            h.await.unwrap().unwrap();
            check_cover(&chunks, &data);
            // Only the walker puts a 300_000-byte frame in its own chunk at offset 28.
            let framed = chunks.iter().any(|c| c.offset == 28 && c.size == 300_000);
            assert_eq!(framed, want_frames, "{name}");
        }
    }

    #[test]
    #[ignore] // needs test-files/video-implementation
    fn real_fixture_u0_has_509_standalone_frames() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-files/video-implementation/NLE-pairs");
        for (name, frames) in [
            ("U0_v1.mov", Some(509usize)),
            ("U1_trim.mov", None),
            ("U2_insert.mov", None),
        ] {
            let Ok(mut f) = std::fs::File::open(p.join(name)) else {
                eprintln!("skipping {name}: fixture absent");
                continue;
            };
            let plan = Mp4Plan::build(&mut f).unwrap();
            if let Some(n) = frames {
                assert_eq!(plan.frame_count(), n, "{name}");
            }
            let (mut at, mut max) = (0u64, 0usize);
            plan.run(&mut f, 20260716, &mut |c| {
                assert_eq!(c.offset, at);
                at += c.size as u64;
                max = max.max(c.size);
                Ok(())
            })
            .unwrap();
            assert_eq!(at, f.metadata().unwrap().len());
            assert!(max as u64 <= MAX_CHUNK);
        }
    }
    #[test]
    #[ignore] // prints a chunk-size histogram for real fixtures
    fn fixture_chunk_size_histogram() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-files");
        let files = [
            "video-implementation/NLE-pairs/U0_v1.mov",
            "video-implementation/NLE-pairs/U2_insert.mov",
            "video-implementation/NLE-pairs/U5_v1.mov",
            "videos/bbb_v1.mp4",
        ];
        for name in files {
            let Ok(mut f) = std::fs::File::open(root.join(name)) else {
                continue;
            };
            let plan = Mp4Plan::build(&mut f).unwrap();
            let mut h = [0usize; 5]; // <4K, 4K-64K, 64K-256K, 256K-1M, 1M-4M(+)
            plan.run(&mut f, 20260716, &mut |c| {
                let b = match c.size {
                    0..4096 => 0,
                    4096..65536 => 1,
                    65536..262144 => 2,
                    262144..1048576 => 3,
                    _ => 4,
                };
                h[b] += 1;
                Ok(())
            })
            .unwrap();
            eprintln!(
                "HIST {name}: <4K={} 4K-64K={} 64K-256K={} 256K-1M={} 1M-4M={}",
                h[0], h[1], h[2], h[3], h[4]
            );
        }
    }
    #[test]
    #[ignore] // measurement: zstd-fast cost on ProRes frame chunks (use test profile numbers with care)
    fn fixture_prores_zstd_cost() {
        use mediagit_compression::{ChunkCodecHint, SmartCompressor, TypeAwareCompressor};
        use std::time::Instant;
        // ZSTDCOST_FILE: path relative to test-files/ (default: Resolve ProRes HQ).
        let rel = std::env::var("ZSTDCOST_FILE")
            .unwrap_or_else(|_| "video-implementation/NLE-pairs/U0_v1.mov".into());
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-files")
            .join(rel);
        let Ok(mut f) = std::fs::File::open(p) else {
            return;
        };
        let plan = Mp4Plan::build(&mut f).unwrap();
        let mut frames: Vec<Vec<u8>> = Vec::new();
        plan.run(&mut f, 20260716, &mut |c| {
            if c.chunk_type == ChunkType::VideoStream
                && c.codec_hint == CodecHint::ProRes
                && frames.len() < 200
            {
                frames.push(c.data);
            }
            Ok(())
        })
        .unwrap();
        let raw: usize = frames.iter().map(Vec::len).sum();
        let sc = SmartCompressor::new();
        let t = Instant::now();
        let comp: Vec<Vec<u8>> = frames
            .iter()
            .map(|d| {
                sc.compress_by_codec(d, ChunkCodecHint::IntraOnlyVideo)
                    .unwrap()
                    .unwrap()
            })
            .collect();
        let ct = t.elapsed().as_secs_f64();
        let csize: usize = comp.iter().map(Vec::len).sum();
        let t = Instant::now();
        let mut out = 0usize;
        for c in &comp {
            out += sc.decompress_typed(c).unwrap().len();
        }
        let dt = t.elapsed().as_secs_f64();
        assert_eq!(out, raw);
        let t = Instant::now();
        let copied: usize = frames.iter().map(|d| d.to_vec().len()).sum();
        let mt = t.elapsed().as_secs_f64();
        let mb = raw as f64 / 1e6;
        eprintln!(
            "ZSTDCOST frames={} raw={:.1}MB comp={:.1}MB ratio={:.4} compress={:.0}MB/s decompress={:.0}MB/s copy={:.0}MB/s ({})",
            frames.len(),
            mb,
            csize as f64 / 1e6,
            csize as f64 / raw as f64,
            mb / ct,
            mb / dt,
            mb / mt.max(1e-9),
            copied
        );
    }
}
