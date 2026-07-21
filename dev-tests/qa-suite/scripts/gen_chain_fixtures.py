"""Generate realistic edit chains from existing test-files assets (read-only sources).

Produces (fixtures-synthetic/chains/):
  photo_v1..v5.jpg   from test-files JPEG: v2 color-curve, v3 text overlay,
                     v4 crop+resize-back, v5 slight rotate + re-encode
  render_v1..v5.png  from test-files PNG: same style of designer edits
  map_v1..v5.svg     from test-files SVG: text edits (add elements, change attrs)
  aria_v1..v5.wav    from test-files WAV: v2 gain, v3 fade-in, v4 +2s silence, v5 trim
  aria_v1..v5.flac   same chain re-exported as FLAC
  car_v1..v3.glb     from test-files GLB: v2 rename node, v3 modify material factor

Produces (fixtures-synthetic/ml/):
  data_v1..v3.parquet  v1 = ~20MB table, v2 = append 10% rows, v3 = append 10% rows + rewrite 1 column

Sources in test-files/ are never modified.
"""
import os
import shutil
import numpy as np
import soundfile as sf
from PIL import Image, ImageDraw, ImageEnhance
import pygltflib
import pyarrow as pa
import pyarrow.parquet as pq

ROOT = "D:/own/saas/mediagit-core"
TF = os.environ.get("MG_QA_TESTFILES", os.path.join(ROOT, "test-files"))
OUT = os.path.join(os.environ.get("MG_QA_FIXTURES", os.path.join(ROOT, "dev-tests", "qa-suite", "fixtures-synthetic")), "chains")
os.makedirs(OUT, exist_ok=True)

SRC_JPG = os.path.join(TF, "3-Modell_St._Mari_Kirche_auf_Hiro-Marker_(linke_Seite).jpg")
SRC_PNG = os.path.join(TF, "3D_model_of_Shucaris_ankylosskelos_appendage.png")
SRC_SVG = os.path.join(TF, "3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg")
SRC_WAV = os.path.join(TF, "_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav")
SRC_GLB = os.path.join(TF, "1965_ac_shelby_427_cobra_sc.glb")


def p(name):
    return os.path.join(OUT, name)


def raster_chain(src, stem, fmt, save_kwargs):
    """v1 = re-encode of source; v2..v5 = cumulative designer edits."""
    img = Image.open(src).convert("RGB")
    img.save(p(f"{stem}_v1.{fmt}"), **save_kwargs)

    v2 = ImageEnhance.Color(ImageEnhance.Contrast(img).enhance(1.15)).enhance(1.2)  # color curve
    v2.save(p(f"{stem}_v2.{fmt}"), **save_kwargs)

    v3 = v2.copy()
    d = ImageDraw.Draw(v3)
    d.text((30, 30), "APPROVED - rev3", fill=(255, 40, 40))
    d.rectangle([20, 20, 220, 60], outline=(255, 40, 40), width=3)
    v3.save(p(f"{stem}_v3.{fmt}"), **save_kwargs)

    w, h = v3.size
    v4 = v3.crop((int(w * 0.05), int(h * 0.05), int(w * 0.95), int(h * 0.95))).resize((w, h), Image.LANCZOS)
    v4.save(p(f"{stem}_v4.{fmt}"), **save_kwargs)

    v5 = v4.rotate(1.5, resample=Image.BICUBIC, expand=False, fillcolor=(0, 0, 0))
    v5.save(p(f"{stem}_v5.{fmt}"), **save_kwargs)


def svg_chain():
    with open(SRC_SVG, encoding="utf-8") as f:
        svg = f.read()
    open(p("map_v1.svg"), "w", encoding="utf-8").write(svg)

    # v2: add a comment + a rect annotation just after the opening <svg ...> tag
    idx = svg.index(">", svg.index("<svg")) + 1
    v2 = svg[:idx] + '\n<!-- rev2: survey annotation -->\n<rect x="10" y="10" width="120" height="40" fill="none" stroke="red" stroke-width="2"/>' + svg[idx:]
    open(p("map_v2.svg"), "w", encoding="utf-8").write(v2)

    # v3: add a text label
    v3 = v2[:idx] + '\n<text x="20" y="70" font-size="24" fill="blue">Gallery survey rev3</text>' + v2[idx:]
    open(p("map_v3.svg"), "w", encoding="utf-8").write(v3)

    # v4: change a global attribute (stroke color swap across doc)
    v4 = v3.replace('stroke="red"', 'stroke="green"').replace("#000000", "#111111")
    open(p("map_v4.svg"), "w", encoding="utf-8").write(v4)

    # v5: add a group of circles (new elements)
    circles = "\n".join(f'<circle cx="{50+i*30}" cy="120" r="8" fill="orange"/>' for i in range(6))
    v5 = v4[:idx] + f'\n<g id="rev5-markers">{circles}</g>' + v4[idx:]
    open(p("map_v5.svg"), "w", encoding="utf-8").write(v5)


def audio_chain():
    data, sr = sf.read(SRC_WAV, dtype="float32")  # (n, 2)
    subtype = "PCM_24"  # ponytail: re-export at 24-bit; source is PCM_32 but float32 pipeline caps at 24-bit fidelity anyway

    versions = {}
    versions[1] = data
    versions[2] = np.clip(data * 1.3, -1.0, 1.0)                     # gain change
    v3 = versions[2].copy()
    n_fade = int(sr * 3)
    v3[:n_fade] *= np.linspace(0, 1, n_fade)[:, None]                # 3s fade-in
    versions[3] = v3
    versions[4] = np.vstack([v3, np.zeros((sr * 2, 2), np.float32)])  # +2s silence
    versions[5] = versions[4][sr * 10 : -sr * 5]                      # trim 10s head / 5s tail

    for ver, d in versions.items():
        sf.write(p(f"aria_v{ver}.wav"), d, sr, subtype=subtype)
        sf.write(p(f"aria_v{ver}.flac"), d, sr, subtype=subtype)


def gltf_chain():
    shutil.copyfile(SRC_GLB, p("car_v1.glb"))

    g = pygltflib.GLTF2().load(SRC_GLB)
    # v2: rename a node (metadata-only edit)
    g.nodes[3].name = "Cobra_TrunkBody_renamed_rev2"
    g.save(p("car_v2.glb"))

    # v3: also modify a material baseColorFactor (regrade-style edit)
    g2 = pygltflib.GLTF2().load(p("car_v2.glb"))
    g2.materials[0].pbrMetallicRoughness.baseColorFactor = [0.8, 0.1, 0.1, 1.0]
    g2.save(p("car_v3.glb"))

    # validity check: reload both
    for f in ("car_v2.glb", "car_v3.glb"):
        r = pygltflib.GLTF2().load(p(f))
        assert r.nodes and r.materials, f


def parquet_chain():
    """v1 = ~20MB table; v2 = append 10% rows; v3 = append 10% rows + rewrite 1 column."""
    # ponytail: deterministic RNG seeded 42; table size ~20MB to match typical data science workflows
    rng = np.random.default_rng(42)
    n_features = 10
    row_bytes = 8 + n_features * 4 + 4 + 32  # id(int64) + features(float32) + label(float32) + text(~32 bytes)
    base_rows = int(20 * 1024 * 1024 / row_bytes)  # ~20 MB target

    ml_dir = os.path.join(os.path.dirname(OUT), "ml")
    os.makedirs(ml_dir, exist_ok=True)

    def pm(name):
        return os.path.join(ml_dir, name)

    def make_table(n_rows, start_id, label_seed):
        cols = {"id": np.arange(start_id, start_id + n_rows, dtype=np.int64)}
        for i in range(n_features):
            cols[f"feature_{i}"] = rng.standard_normal(n_rows).astype(np.float32)
        label_rng = np.random.default_rng(label_seed)
        cols["label"] = label_rng.standard_normal(n_rows).astype(np.float32)
        cols["metadata"] = [f"row_{i}_v{label_seed // 100}" for i in range(n_rows)]
        return pa.table(cols)

    # v1: base table
    t1 = make_table(base_rows, 0, 100)
    pq.write_table(t1, pm("data_v1.parquet"), compression="snappy")
    v1_size = os.path.getsize(pm("data_v1.parquet")) / (1024 * 1024)

    # v2: append 10% rows (realistic data ingestion)
    extra_rows_v2 = int(base_rows * 0.10)
    t2_extra = make_table(extra_rows_v2, base_rows, 200)
    t2 = pa.concat_tables([t1, t2_extra])
    pq.write_table(t2, pm("data_v2.parquet"), compression="snappy")
    v2_size = os.path.getsize(pm("data_v2.parquet")) / (1024 * 1024)

    # v3: append 10% more rows + rewrite label column (simulates model retraining)
    extra_rows_v3 = int(base_rows * 0.10)
    t3_extra = make_table(extra_rows_v3, base_rows + extra_rows_v2, 300)
    t3 = pa.concat_tables([t2, t3_extra])
    new_label = np.random.default_rng(999).standard_normal(t3.num_rows).astype(np.float32)
    label_idx = t3.schema.get_field_index("label")
    t3 = t3.set_column(label_idx, "label", pa.array(new_label))
    pq.write_table(t3, pm("data_v3.parquet"), compression="snappy")
    v3_size = os.path.getsize(pm("data_v3.parquet")) / (1024 * 1024)


def demo():
    import hashlib

    def sha(path):
        h = hashlib.sha256()
        with open(path, "rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()

    for stem, n in [("photo", 5), ("render", 5), ("map", 5), ("aria", 5), ("car", 3)]:
        ext = {"photo": "jpg", "render": "png", "map": "svg", "aria": "wav", "car": "glb"}[stem]
        hashes = [sha(p(f"{stem}_v{i}.{ext}")) for i in range(1, n + 1)]
        assert len(set(hashes)) == n, f"{stem}: duplicate versions"
    # every FLAC decodes
    for i in range(1, 6):
        d, _ = sf.read(p(f"aria_v{i}.flac"))
        assert len(d) > 0
    # every raster reopens
    for f in os.listdir(OUT):
        if f.endswith((".jpg", ".png")):
            Image.open(p(f)).verify()
    # parquet: v1..v3 exist and differ
    ml_dir = os.path.join(os.path.dirname(OUT), "ml")
    if os.path.exists(ml_dir):
        for i in range(1, 4):
            fpath = os.path.join(ml_dir, f"data_v{i}.parquet")
            if os.path.exists(fpath):
                assert pq.read_table(fpath).num_rows > 0, f"data_v{i}.parquet is empty"
        h1 = sha(os.path.join(ml_dir, "data_v1.parquet"))
        h2 = sha(os.path.join(ml_dir, "data_v2.parquet"))
        h3 = sha(os.path.join(ml_dir, "data_v3.parquet"))
        assert h1 != h2 and h2 != h3, "parquet: versions should differ"
    print("[demo] chains self-check OK")


if __name__ == "__main__":
    raster_chain(SRC_JPG, "photo", "jpg", {"quality": 90})
    raster_chain(SRC_PNG, "render", "png", {})
    svg_chain()
    audio_chain()
    gltf_chain()
    parquet_chain()
    demo()
