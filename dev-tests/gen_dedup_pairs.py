# gen_dedup_pairs.py - regenerate dev-tests/dedup-pairs/ fixtures.
#
# The original pairs (2026-07-07) were hand-made and lost in a cleanup; this
# script makes the fixtures reproducible. Pairs are consumed by
# crates/mediagit-versioning/examples/dedup_report.rs (delta-hit-rate gate).
#
# v1 = byte-identical copy of a test-files/ source.
# v2 = v1 with a deterministic "edit": 4 KiB overwritten at 25% offset,
#      1 KiB inserted at 60% offset (CDC shift-resistance), 16 KiB appended.
#      All injected bytes come from random.Random(0) - fully deterministic.
# parquet/safetensors pairs are copied from the qa-suite synthetic ML
# fixtures, which already model realistic v1->v2 edits.
#
# NOTE: regenerating pairs invalidates dev-tests/dedup-baseline.json - re-lock
# it by saving a fresh dedup_report run (baseline re-locked 2026-07-19).
#
# Usage: python dev-tests/gen_dedup_pairs.py   (from anywhere; paths are
#        resolved relative to this script's parent = repo root/dev-tests)

import random
import shutil
from pathlib import Path

DEV_TESTS = Path(__file__).resolve().parent
REPO = DEV_TESTS.parent
TF = REPO / "test-files"
OUT = DEV_TESTS / "dedup-pairs"

# base name -> source file (v1 copied as-is, v2 edited)
EDIT_SOURCES = {
    "wav": TF / "_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav",
    "flac": TF / "_Amir_Tangsiri__Dokhtare_Koli.flac",
    "fbx": TF / "56-fbx/fbx/Dragon 2.5_fbx.fbx",
    "fbxfair": TF / "56-fbx/fbx/Dragon_Baked_Actions_fbx_7.4_binary.fbx",
    "stl": TF / "39-stl/stl/Dragon 2.5_stl.stl",
    "blend": TF / "27-blender/blender/Dragon_2.5_For_Animations.blend",
    "ai": TF / "12690118_5053480.ai",
    "ply": TF / "93-ply/ply/Dragon 2.5_ply.ply",
    # Added 2026-07-28. psd/mov/mkv previously had NO pair, so the dedup report
    # showed 0% for them — an arithmetic consequence of having one version to
    # compare against, not a chunker result. Every format with a pair showed
    # real dedup; every format without one showed zero. The absence was being
    # read as a pipeline weakness.
    "psd": TF / "psd/26952784_food_flyer_19.psd",
    # Smallest real .mkv available. No paired mkv variant exists, so this uses
    # the synthetic edit — which models a container/metadata change, not a
    # re-encode. That distinction matters: a re-encode changes every byte and
    # cannot dedup by construction, so measuring one would prove nothing.
    "mkv": TF / "video-variants/bbb-5s-h264.mkv",
}

# base name -> (v1 source, v2 source) copied verbatim
COPY_SOURCES = {
    "parquet": (
        DEV_TESTS / "qa-suite/fixtures-synthetic/ml/data_v1.parquet",
        DEV_TESTS / "qa-suite/fixtures-synthetic/ml/data_v2.parquet",
    ),
    "safetensors": (
        DEV_TESTS / "qa-suite/fixtures-synthetic/ml/model_v1.safetensors",
        DEV_TESTS / "qa-suite/fixtures-synthetic/ml/model_v2.safetensors",
    ),
    # Real-world video variants beat a synthetic edit here: these are the same
    # footage with a genuine metadata change and a genuine remux, i.e. the
    # workflows where dedup *should* pay off. Container-aware chunking aligns
    # boundaries to atoms/elements, so an untouched media payload ought to be
    # reused almost entirely.
    "mov": (
        TF / "video-variants/bbb-5s-h264.mov",
        TF / "video-variants/bbb-5s-meta.mov",
    ),
    "mp4remux": (
        TF / "101394-video-720.mp4",
        TF / "video-variants/remux-faststart.mp4",
    ),
    # Negative control: a re-encode shares no bytes with its source. This pair
    # SHOULD show ~0% dedup. Without it, a genuine 0% on video is
    # indistinguishable from the missing-fixture artifact this change fixes.
    "mp4reencode": (
        TF / "101394-video-720.mp4",
        TF / "video-variants/h265-reencode.mp4",
    ),
}


def edited_v2(data: bytes) -> bytes:
    rng = random.Random(0)
    buf = bytearray(data)
    n = len(buf)
    over_at, over_len = n // 4, 4096
    buf[over_at : over_at + over_len] = bytes(rng.randrange(256) for _ in range(over_len))
    ins_at = (n * 6) // 10
    insert = bytes(rng.randrange(256) for _ in range(1024))
    tail = bytes(rng.randrange(256) for _ in range(16384))
    return bytes(buf[:ins_at]) + insert + bytes(buf[ins_at:]) + tail


def main() -> None:
    OUT.mkdir(exist_ok=True)
    for base, src in EDIT_SOURCES.items():
        ext = src.suffix
        data = src.read_bytes()
        (OUT / f"{base}_v1{ext}").write_bytes(data)
        (OUT / f"{base}_v2{ext}").write_bytes(edited_v2(data))
        print(f"{base}: {len(data)} bytes -> pair written")
    for base, (v1, v2) in COPY_SOURCES.items():
        ext = v1.suffix
        shutil.copyfile(v1, OUT / f"{base}_v1{ext}")
        shutil.copyfile(v2, OUT / f"{base}_v2{ext}")
        print(f"{base}: copied synthetic v1/v2")
    print(f"done -> {OUT}")


if __name__ == "__main__":
    main()
