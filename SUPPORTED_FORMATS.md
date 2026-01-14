# Supported File Formats

MediaGit supports **100+ file formats** with optimized chunking, compression, and deduplication.

## Quick Reference

| Category | Chunking | Compression | Best For |
|----------|----------|-------------|----------|
| Video/Audio (Media) | MediaAware | Store | Cross-version dedup |
| Text/Code/Data | Rolling CDC | Brotli | Incremental edits |
| ML Data/Models | Rolling CDC | Zstd | Fine-tuning dedup |
| Images (lossy) | Fixed | Store | Exact match dedup |
| Archives | Fixed | Store | Exact match dedup |

---

## Video (MediaAware Chunking)

Structure-aware chunking at atom/cluster boundaries.

| Extension | Format |
|-----------|--------|
| `.mp4`, `.m4v`, `.m4a` | MPEG-4 |
| `.mov`, `.qt` | QuickTime |
| `.mkv`, `.mka`, `.mk3d` | Matroska |
| `.webm` | WebM |
| `.avi`, `.riff` | AVI/RIFF |
| `.flv`, `.wmv`, `.mpg` | Legacy |

---

## Audio

| Extension | Chunking | Compression |
|-----------|----------|-------------|
| `.wav` | MediaAware | Zstd |
| `.flac`, `.aiff`, `.alac` | Rolling CDC | Zstd |
| `.mp3`, `.aac`, `.ogg`, `.opus` | Fixed | Store |

---

## Images

| Type | Extensions | Compression |
|------|------------|-------------|
| **Compressed** | jpg, png, gif, webp, avif, heic | Store |
| **Raw/Uncompressed** | tiff, bmp, psd, raw, cr2, nef, exr, hdr | Zstd Best |
| **GPU Textures** | dds, ktx, astc, pvr, basis | Store |

---

## 3D Assets

| Extension | Format | Chunking |
|-----------|--------|----------|
| `.glb`, `.gltf` | glTF | MediaAware |
| `.obj`, `.fbx`, `.blend` | Other | Rolling CDC |

---

## Text/Code (Brotli Best)

Rolling CDC for maximum incremental deduplication.

**Extensions:** txt, md, rst, csv, tsv, json, xml, html, yaml, yml, toml, ini, cfg, sql, graphql, proto, rs, py, js, ts, go, java, c, cpp, h

---

## ML Data Formats

Rolling CDC for dataset versioning.

| Extension | Format | Compression |
|-----------|--------|-------------|
| `.parquet`, `.arrow`, `.feather`, `.orc`, `.avro` | Columnar | Store |
| `.hdf5`, `.h5`, `.nc`, `.netcdf` | HDF5/NetCDF | Zstd |
| `.npy`, `.npz` | NumPy | Zstd |
| `.tfrecords`, `.petastorm` | TensorFlow/Uber | Zstd |

---

## ML Model Formats

Rolling CDC for fine-tuning deduplication.

| Extension | Framework | Compression |
|-----------|-----------|-------------|
| `.pt`, `.pth`, `.bin` | PyTorch | Zstd |
| `.ckpt`, `.pb` | TensorFlow | Zstd |
| `.safetensors` | Hugging Face | Zstd |
| `.pkl`, `.joblib` | Scikit-learn | Zstd |

---

## ML Deployment Formats

Rolling CDC for model versioning.

| Extension | Format |
|-----------|--------|
| `.onnx` | ONNX (cross-framework) |
| `.gguf`, `.ggml` | LLM inference |
| `.tflite` | TensorFlow Lite |
| `.mlmodel`, `.coreml` | Apple CoreML |
| `.keras`, `.pte` | Keras/ExecuTorch |
| `.mleap`, `.pmml` | Interchange |
| `.llamafile` | Executable LLM |

---

## Documents

| Extension | Chunking | Compression |
|-----------|----------|-------------|
| `.pdf`, `.svg`, `.eps`, `.ai` | Rolling CDC | Zstd/Brotli |

---

## Archives (Store Only)

| Extension | Format |
|-----------|--------|
| `.zip`, `.7z`, `.rar` | Compressed archives |
| `.gz`, `.xz`, `.bz2` | Compression streams |
| `.tar` | Uncompressed container (Zstd applied) |

---

## Strategy Summary

| Strategy | When Used | Benefit |
|----------|-----------|---------|
| **MediaAware** | Video, Audio, 3D | Parses structure for cross-version dedup |
| **Rolling CDC** | Text, ML, Documents | Content-defined boundaries for incremental dedup |
| **Fixed** | Images, Archives | Simple chunking for exact-match dedup |
| **Brotli** | Text/Code | 85-93% compression |
| **Zstd** | Binary, ML | 30-60% compression |
| **Store** | Pre-compressed | Pass-through (0%) |
