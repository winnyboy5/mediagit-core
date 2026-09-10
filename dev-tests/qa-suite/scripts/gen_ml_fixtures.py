"""Generate ML-engineer fixture set (edit chains) for MediaGit delta-compression testing.

Produces:
  model_v1..v5.safetensors    ~150MB each, ~30% of tensors gaussian-perturbed per version
  checkpoint_v1..v3.npz       ~50MB each, np.savez (uncompressed)
  checkpoint_v1_compressed.npz  same v1 data via np.savez_compressed (size comparison only)
  training_data_v1..v3.parquet  ~80MB base, v2 +10% rows, v3 +10% rows and rewrites 1 column
  model_v1..v2.onnx           ~30MB float initializers, v2 = same graph w/ perturbed weights

Deterministic (seeded) so the fixture set is reproducible.
"""
import os
import numpy as np
from safetensors.numpy import save_file
import pyarrow as pa
import pyarrow.parquet as pq
import onnx
from onnx import helper, numpy_helper, TensorProto

OUT = os.path.join(os.environ.get("MG_QA_FIXTURES", os.path.join(os.path.dirname(__file__), "..", "fixtures-synthetic")), "ml")
os.makedirs(OUT, exist_ok=True)

MB = 1024 * 1024
# MG_QA_SCALE multiplies the synthetic tensor/dataset sizes (SCALE tier, phase 10).
# Default 1 keeps the STANDARD/STRESS sizes exactly as before.
SCALE = max(1, int(os.environ.get("MG_QA_SCALE", "1")))


def p(name):
    return os.path.join(OUT, name)


# ---------------------------------------------------------------------------
# 1. model.safetensors v1..v5 (~150MB, dict of float32 tensors)
# ---------------------------------------------------------------------------
def gen_safetensors():
    target_bytes = 150 * MB * SCALE
    rng = np.random.default_rng(42)

    # Fixed tensor shapes mimicking a small model's named parameters.
    shapes = {
        "embedding.weight": (6000, 1024),
        "layer.0.weight": (2048, 2048),
        "layer.1.weight": (2048, 2048),
        "layer.2.weight": (2048, 2048),
        "layer.3.weight": (2048, 2048),
        "layer.0.bias": (2048,),
        "layer.1.bias": (2048,),
        "layer.2.bias": (2048,),
        "layer.3.bias": (2048,),
        "head.weight": (1024, 512),
        "head.bias": (512,),
    }
    used = sum(np.prod(s) for s in shapes.values()) * 4
    # filler tensor to land close to target_bytes
    remaining_elems = max(0, (target_bytes - used) // 4)
    if remaining_elems > 0:
        side = int(remaining_elems)
        shapes["filler.weight"] = (side,)

    v1 = {name: rng.standard_normal(shape, dtype=np.float32) for name, shape in shapes.items()}
    total = sum(t.nbytes for t in v1.values())
    print(f"[safetensors] tensor count={len(v1)} total={total/MB:.1f} MB")
    save_file(v1, p("model_v1.safetensors"))

    prev = v1
    names = list(shapes.keys())
    for ver in range(2, 6):
        cur = {k: v.copy() for k, v in prev.items()}
        subset_rng = np.random.default_rng(100 + ver)
        n_noisy = max(1, round(len(names) * 0.30))
        noisy_names = subset_rng.choice(names, size=n_noisy, replace=False)
        for name in noisy_names:
            noise = subset_rng.normal(0, 0.01, size=cur[name].shape).astype(np.float32)
            cur[name] = cur[name] + noise
        save_file(cur, p(f"model_v{ver}.safetensors"))
        print(f"[safetensors] v{ver}: perturbed {list(noisy_names)}")
        prev = cur


# ---------------------------------------------------------------------------
# 2. checkpoint.npz v1..v3 (~50MB) + one compressed variant of v1
# ---------------------------------------------------------------------------
def gen_checkpoint_npz():
    target_bytes = 50 * MB * SCALE
    rng = np.random.default_rng(7)
    shapes = {
        "opt_state.m": (3000, 1024),
        "opt_state.v": (3000, 1024),
        "step_weights": (2000, 1024),
    }
    used = sum(np.prod(s) for s in shapes.values()) * 4
    remaining_elems = max(0, (target_bytes - used) // 4)
    if remaining_elems > 0:
        shapes["filler"] = (int(remaining_elems),)

    v1 = {name: rng.standard_normal(shape, dtype=np.float32) for name, shape in shapes.items()}
    total = sum(a.nbytes for a in v1.values())
    print(f"[npz] array count={len(v1)} total={total/MB:.1f} MB")
    np.savez(p("checkpoint_v1.npz"), **v1)
    np.savez_compressed(p("checkpoint_v1_compressed.npz"), **v1)  # size-comparison only, not chain

    prev = v1
    for ver in range(2, 4):
        cur = {k: v.copy() for k, v in prev.items()}
        subset_rng = np.random.default_rng(200 + ver)
        names = list(shapes.keys())
        n_noisy = max(1, round(len(names) * 0.30))
        noisy_names = subset_rng.choice(names, size=n_noisy, replace=False)
        for name in noisy_names:
            noise = subset_rng.normal(0, 0.01, size=cur[name].shape).astype(np.float32)
            cur[name] = cur[name] + noise
        np.savez(p(f"checkpoint_v{ver}.npz"), **cur)
        prev = cur


# ---------------------------------------------------------------------------
# 3. training_data.parquet v1..v3 (~80MB base)
# ---------------------------------------------------------------------------
def gen_parquet():
    rng = np.random.default_rng(11)
    n_features = 20
    row_bytes = 8 + n_features * 4 + 4  # id(int64) + features(float32) + label(float32)
    base_rows = int(80 * MB * SCALE / row_bytes)

    def make_table(n_rows, start_id, label_rng):
        cols = {"id": np.arange(start_id, start_id + n_rows, dtype=np.int64)}
        for i in range(n_features):
            cols[f"feature_{i}"] = rng.standard_normal(n_rows).astype(np.float32)
        cols["label"] = label_rng.standard_normal(n_rows).astype(np.float32)
        return pa.table(cols)

    t1 = make_table(base_rows, 0, np.random.default_rng(21))
    pq.write_table(t1, p("training_data_v1.parquet"), compression="snappy")
    print(f"[parquet] v1 rows={base_rows} bytes={os.path.getsize(p('training_data_v1.parquet'))/MB:.1f} MB")

    extra_rows_v2 = int(base_rows * 0.10)
    t2_extra = make_table(extra_rows_v2, base_rows, np.random.default_rng(22))
    t2 = pa.concat_tables([t1, t2_extra])
    pq.write_table(t2, p("training_data_v2.parquet"), compression="snappy")
    print(f"[parquet] v2 rows={t2.num_rows} bytes={os.path.getsize(p('training_data_v2.parquet'))/MB:.1f} MB")

    extra_rows_v3 = int(base_rows * 0.10)
    t3_extra = make_table(extra_rows_v3, base_rows + extra_rows_v2, np.random.default_rng(23))
    t3 = pa.concat_tables([t2, t3_extra])
    # rewrite the label column entirely (simulates a full label-recompute pass)
    new_label = np.random.default_rng(999).standard_normal(t3.num_rows).astype(np.float32)
    label_idx = t3.schema.get_field_index("label")
    t3 = t3.set_column(label_idx, "label", pa.array(new_label))
    pq.write_table(t3, p("training_data_v3.parquet"), compression="snappy")
    print(f"[parquet] v3 rows={t3.num_rows} bytes={os.path.getsize(p('training_data_v3.parquet'))/MB:.1f} MB")


# ---------------------------------------------------------------------------
# 4. model.onnx v1..v2 (~30MB float initializers)
# ---------------------------------------------------------------------------
def gen_onnx():
    rng = np.random.default_rng(55)
    dim = 1000
    n_layers = 7  # 1000x1000 float32 = 4MB each -> ~28MB
    batch = "N"

    def build_graph(weight_arrays, bias_arrays, out_dim_last):
        nodes = []
        initializers = []
        cur_name = "input"
        for i, (w, b) in enumerate(zip(weight_arrays, bias_arrays)):
            w_name, b_name = f"W{i}", f"B{i}"
            initializers.append(numpy_helper.from_array(w, name=w_name))
            initializers.append(numpy_helper.from_array(b, name=b_name))
            mm_out = f"mm{i}"
            add_out = f"add{i}"
            relu_out = f"relu{i}" if i < len(weight_arrays) - 1 else "output"
            nodes.append(helper.make_node("MatMul", [cur_name, w_name], [mm_out]))
            nodes.append(helper.make_node("Add", [mm_out, b_name], [add_out]))
            nodes.append(helper.make_node("Relu", [add_out], [relu_out]))
            cur_name = relu_out

        graph_input = helper.make_tensor_value_info("input", TensorProto.FLOAT, [batch, dim])
        graph_output = helper.make_tensor_value_info("output", TensorProto.FLOAT, [batch, out_dim_last])
        graph = helper.make_graph(nodes, "synthetic_ml_model", [graph_input], [graph_output], initializers)
        model = helper.make_model(graph, producer_name="mediagit-fixture-gen")
        model.opset_import[0].version = 17
        onnx.checker.check_model(model)
        return model

    weights_v1, biases_v1 = [], []
    for i in range(n_layers):
        out_dim = dim if i < n_layers - 1 else 500
        weights_v1.append(rng.standard_normal((dim, out_dim), dtype=np.float32))
        biases_v1.append(rng.standard_normal((out_dim,), dtype=np.float32))
        dim = out_dim

    model_v1 = build_graph(weights_v1, biases_v1, out_dim_last=weights_v1[-1].shape[1])
    onnx.save(model_v1, p("model_v1.onnx"))
    print(f"[onnx] v1 bytes={os.path.getsize(p('model_v1.onnx'))/MB:.1f} MB")

    noise_rng = np.random.default_rng(56)
    weights_v2 = [w + noise_rng.normal(0, 0.01, size=w.shape).astype(np.float32) for w in weights_v1]
    biases_v2 = [b + noise_rng.normal(0, 0.01, size=b.shape).astype(np.float32) for b in biases_v1]
    model_v2 = build_graph(weights_v2, biases_v2, out_dim_last=weights_v2[-1].shape[1])
    onnx.save(model_v2, p("model_v2.onnx"))
    print(f"[onnx] v2 bytes={os.path.getsize(p('model_v2.onnx'))/MB:.1f} MB")


def demo():
    """ponytail: smallest possible self-check — assert files exist & versioned files differ."""
    import hashlib

    def sha(path):
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for chunk in iter(lambda: f.read(1 << 20), b""):
                h.update(chunk)
        return h.hexdigest()

    assert os.path.exists(p("model_v1.safetensors"))
    assert sha(p("model_v1.safetensors")) != sha(p("model_v2.safetensors"))
    assert os.path.exists(p("checkpoint_v1_compressed.npz"))
    assert os.path.getsize(p("checkpoint_v1_compressed.npz")) < os.path.getsize(p("checkpoint_v1.npz"))
    assert sha(p("training_data_v1.parquet")) != sha(p("training_data_v2.parquet"))
    assert sha(p("model_v1.onnx")) != sha(p("model_v2.onnx"))
    print("[demo] ML fixture self-check OK")


if __name__ == "__main__":
    gen_safetensors()
    gen_checkpoint_npz()
    gen_parquet()
    gen_onnx()
    demo()
