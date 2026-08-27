"""Generate the SCALE-tier many-small-files corpus for MediaGit phase 10.

Produces (fixtures-synthetic/scale/manyfiles/):
  A nested tree of MG_QA_FILECOUNT seeded files (default 10000), 0.5-8 KB each,
  split PER_DIR files per leaf directory. Exercises index / tree-diff / staging at
  file-COUNT scale (the axis the size-only STRESS tier never touched).

  ~1 in 7 files reuses a fixed shared block so the corpus also carries real dedup
  opportunity at count scale; the rest are unique seeded content.

The multi-GB single blobs used by phase 10's resource-pressure / throughput drills are
NOT produced here - they are generated at drill time under work/ so post-phase teardown
reclaims them (persisting 10 GB in fixtures-synthetic/, which is kept by default, would
fight the disk budget).

Deterministic: content is seeded by file index, so a re-run is byte-identical.
gen_manifest.py walks fixtures-synthetic/ and inventories these automatically.
"""
import os
import hashlib
import numpy as np

OUT = os.path.join(
    os.environ.get("MG_QA_FIXTURES", os.path.join(os.path.dirname(__file__), "..", "fixtures-synthetic")),
    "scale", "manyfiles",
)
FILECOUNT = max(1, int(os.environ.get("MG_QA_FILECOUNT", "10000")))
PER_DIR = 200

# Fixed shared block (~2 KB) reused by every 7th file -> count-scale dedup opportunity.
SHARED = np.random.default_rng(0).integers(0, 256, 2048, dtype=np.uint8).tobytes()


def content_for(index):
    """Deterministic bytes for file `index`: shared block for 1-in-7, else unique."""
    if index % 7 == 0:
        return SHARED
    rng = np.random.default_rng(1_000_000 + index)
    size = 512 + (index % 15) * 512  # 512 B .. ~8 KB
    return rng.integers(0, 256, size, dtype=np.uint8).tobytes()


def path_for(index):
    leaf = index // PER_DIR
    d = os.path.join(OUT, "d%05d" % leaf)
    return d, os.path.join(d, "f%07d.bin" % index)


def main():
    made = 0
    last_dir = None
    for i in range(FILECOUNT):
        d, path = path_for(i)
        if d != last_dir:
            os.makedirs(d, exist_ok=True)
            last_dir = d
        with open(path, "wb") as f:
            f.write(content_for(i))
        made += 1
        if made % 2000 == 0:
            print("[scale] wrote %d/%d small files" % (made, FILECOUNT))
    print("[scale] many-files corpus: %d files under %s" % (made, OUT))
    demo()


def demo():
    """ponytail: smallest self-check - count is right, content is deterministic & dedup present."""
    def sha(path):
        h = hashlib.sha256()
        with open(path, "rb") as f:
            h.update(f.read())
        return h.hexdigest()

    n = sum(len(files) for _, _, files in os.walk(OUT))
    assert n == FILECOUNT, "expected %d files, found %d" % (FILECOUNT, n)
    # determinism: regenerating a file's bytes reproduces the on-disk hash
    _, sample = path_for(3)
    tmp = sample + ".chk"
    with open(tmp, "wb") as f:
        f.write(content_for(3))
    assert sha(tmp) == sha(sample), "content_for(3) not deterministic vs on-disk"
    os.remove(tmp)
    # dedup opportunity present: files 0 and 7 share the block
    _, p0 = path_for(0)
    _, p7 = path_for(7)
    assert sha(p0) == sha(p7), "expected shared block for 1-in-7 files"
    print("[demo] scale many-files self-check OK (%d files)" % FILECOUNT)


if __name__ == "__main__":
    main()
