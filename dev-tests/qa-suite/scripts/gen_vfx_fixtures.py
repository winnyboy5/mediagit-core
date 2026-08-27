"""Generate VFX fixture set: EXR frame sequence for MediaGit delta/dedup testing.

Produces (fixtures-synthetic/vfx/):
  shot010/frame_0001.exr .. frame_0120.exr
      1920x1080 half-float RGB, ZIP compression.
      Content = static gradient plate (shared across all frames)
              + moving bright gaussian blob (per-frame position)
              + low-level per-frame noise (sigma 0.002).
  shot010_regrade/frame_0001.exr .. frame_0030.exr
      Same recipe for frames 1-30 but with a global color multiply
      (R*1.10, G*1.02, B*0.92) applied — the "v2 regrade" edit chain.

Deterministic: per-frame noise is seeded by frame index, so the regrade
frames share the exact same plate/blob/noise as the originals (only the
grade differs) — exactly what a real regrade produces.
"""
import os
import numpy as np
import OpenEXR

OUT = os.path.join(os.environ.get("MG_QA_FIXTURES", os.path.join(os.path.dirname(__file__), "..", "fixtures-synthetic")), "vfx")
W, H = 1920, 1080
# MG_QA_SCALE multiplies the sequence length (SCALE tier, phase 10). Default 1
# keeps the STANDARD 120/30-frame sequence exactly as before.
SCALE = max(1, int(os.environ.get("MG_QA_SCALE", "1")))
N_FRAMES = 120 * SCALE
N_REGRADE = 30 * SCALE
GRADE = np.array([1.10, 1.02, 0.92], dtype=np.float32)  # RGB multiply

# Static gradient plate, computed once (float32 workspace, cast to half at write)
_yy, _xx = np.mgrid[0:H, 0:W].astype(np.float32)
PLATE = np.stack(
    [
        _xx / W * 0.6,                       # R: horizontal ramp
        _yy / H * 0.5,                       # G: vertical ramp
        0.3 + 0.2 * np.sin(_xx / W * np.pi), # B: soft horizontal wave
    ],
    axis=-1,
)


def render_frame(idx):
    """Plate + moving bright blob + seeded per-frame noise. Returns float32 HxWx3."""
    img = PLATE.copy()
    # blob sweeps left->right and bobs vertically over the sequence
    t = idx / N_FRAMES
    cx = 100 + t * (W - 200)
    cy = H / 2 + 200 * np.sin(t * 4 * np.pi)
    sigma = 60.0
    blob = np.exp(-(((_xx - cx) ** 2 + (_yy - cy) ** 2) / (2 * sigma**2))) * 2.5
    img += blob[..., None]  # bright white element
    rng = np.random.default_rng(1000 + idx)  # seeded per frame -> reproducible
    img += rng.normal(0, 0.002, size=img.shape).astype(np.float32)
    return img


def write_exr(path, img_f32):
    header = {"compression": OpenEXR.ZIP_COMPRESSION, "type": OpenEXR.scanlineimage}
    OpenEXR.File(header, {"RGB": img_f32.astype(np.float16)}).write(path)


def main():
    shot = os.path.join(OUT, "shot010")
    regrade = os.path.join(OUT, "shot010_regrade")
    os.makedirs(shot, exist_ok=True)
    os.makedirs(regrade, exist_ok=True)

    for idx in range(1, N_FRAMES + 1):
        img = render_frame(idx)
        write_exr(os.path.join(shot, f"frame_{idx:04d}.exr"), img)
        if idx <= N_REGRADE:
            write_exr(os.path.join(regrade, f"frame_{idx:04d}.exr"), img * GRADE)
        if idx % 30 == 0:
            print(f"[vfx] wrote {idx}/{N_FRAMES}")

    # self-check: files valid EXR, regrade differs from original, plate shared
    f1 = OpenEXR.File(os.path.join(shot, "frame_0001.exr")).channels()["RGB"].pixels
    g1 = OpenEXR.File(os.path.join(regrade, "frame_0001.exr")).channels()["RGB"].pixels
    assert f1.shape == (H, W, 3) and f1.dtype == np.float16
    assert not np.array_equal(f1, g1)
    assert np.allclose(np.asarray(g1, np.float32), np.asarray(f1, np.float32) * GRADE, atol=0.02)
    print("[demo] VFX fixture self-check OK")


if __name__ == "__main__":
    main()
