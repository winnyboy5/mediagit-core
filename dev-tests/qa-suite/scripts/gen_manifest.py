"""Write fixtures-synthetic/manifest-synthetic.tsv covering all generated fixtures.

Columns: relative_path, bytes, sha256, chain_id, version
chain_id/version are derived from filenames:
  *_vN.*            -> chain = dir/stem, version = N
  vfx frame files   -> chain = shot dir (whole sequence = one chain,
                       shot010_regrade is the v2 chain of shot010 frames 1-30);
                       version = 1 for shot010, 2 for shot010_regrade;
                       per-frame identity is the filename itself.
"""
import hashlib
import os
import re
from pathlib import Path

# scripts/ -> qa-suite/; the default fixture tree is its sibling. Derived rather than
# hardcoded so the generator runs from any checkout.
_QA_SUITE = Path(__file__).resolve().parents[1]
ROOT = os.environ.get("MG_QA_FIXTURES", str(_QA_SUITE / "fixtures-synthetic"))
MANIFEST = os.path.join(ROOT, "manifest-synthetic.tsv")


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def chain_info(rel):
    d = os.path.dirname(rel).replace("\\", "/")
    base = os.path.basename(rel)
    stem, ext = os.path.splitext(base)
    m = re.match(r"(.+)_v(\d+)(_compressed)?$", stem)
    if d.startswith("vfx/"):
        shot = d.split("/")[1]
        return f"vfx/{shot.removesuffix('_regrade')}", "2" if shot.endswith("_regrade") else "1"
    if m:
        suffix = "_compressed" if m.group(3) else ""
        return f"{d}/{m.group(1)}{ext}{suffix}", m.group(2)
    return f"{d}/{stem}{ext}", "1"


rows = []
for dirpath, _, files in os.walk(ROOT):
    for f in sorted(files):
        full = os.path.join(dirpath, f)
        rel = os.path.relpath(full, ROOT).replace("\\", "/")
        if rel == "manifest-synthetic.tsv":
            continue
        chain, ver = chain_info(rel)
        rows.append((rel, os.path.getsize(full), sha256(full), chain, ver))

rows.sort(key=lambda r: (r[3], int(r[4]), r[0]))
with open(MANIFEST, "w", newline="\n") as f:
    f.write("relative_path\tbytes\tsha256\tchain_id\tversion\n")
    for r in rows:
        f.write("\t".join(str(x) for x in r) + "\n")

total = sum(r[1] for r in rows)
print(f"[manifest] {len(rows)} files, {total/2**30:.2f} GiB -> {MANIFEST}")
assert len(rows) == len({r[0] for r in rows})
assert all(r[1] > 0 for r in rows)
print("[demo] manifest self-check OK")
