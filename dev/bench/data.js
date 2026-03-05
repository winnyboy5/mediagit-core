window.BENCHMARK_DATA = {
  "lastUpdate": 1772692399897,
  "repoUrl": "https://github.com/winnyboy5/mediagit-core",
  "entries": {
    "Benchmark": [
      {
        "commit": {
          "author": {
            "email": "winnyboy5@gmail.com",
            "name": "Aswin Krishnamoorthy",
            "username": "winnyboy5"
          },
          "committer": {
            "email": "noreply@github.com",
            "name": "GitHub",
            "username": "web-flow"
          },
          "distinct": true,
          "id": "feb3a225229fa32e801bda4777b5401cb5065dd8",
          "message": "Refactor/version bumps (#36)\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: bench fix and version bump\n\n* fix: user config",
          "timestamp": "2026-03-05T11:15:50+05:30",
          "tree_id": "6c18a11c869342d407d29296110699713590d78a",
          "url": "https://github.com/winnyboy5/mediagit-core/commit/feb3a225229fa32e801bda4777b5401cb5065dd8"
        },
        "date": 1772692399629,
        "tool": "cargo",
        "benches": [
          {
            "name": "compression/zstd_compress_1kb_text_default",
            "value": 18802,
            "range": "± 71",
            "unit": "ns/iter"
          },
          {
            "name": "compression/brotli_compress_1kb_text_default",
            "value": 958232,
            "range": "± 116333",
            "unit": "ns/iter"
          },
          {
            "name": "compression/zstd_compress_10kb_text_default",
            "value": 16830,
            "range": "± 174",
            "unit": "ns/iter"
          },
          {
            "name": "compression/brotli_compress_10kb_text_default",
            "value": 995849,
            "range": "± 119616",
            "unit": "ns/iter"
          },
          {
            "name": "compression/zstd_compress_100kb_text_default",
            "value": 29576,
            "range": "± 139",
            "unit": "ns/iter"
          },
          {
            "name": "compression/brotli_compress_100kb_text_default",
            "value": 1472998,
            "range": "± 67105",
            "unit": "ns/iter"
          },
          {
            "name": "decompression/zstd_decompress_100kb",
            "value": 34885,
            "range": "± 104",
            "unit": "ns/iter"
          },
          {
            "name": "decompression/brotli_decompress_100kb",
            "value": 182010,
            "range": "± 1270",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/zstd_fast",
            "value": 5576,
            "range": "± 22",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/zstd_default",
            "value": 18784,
            "range": "± 584",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/zstd_best",
            "value": 64790663,
            "range": "± 647365",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/brotli_fast",
            "value": 32587,
            "range": "± 144",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/brotli_default",
            "value": 1004323,
            "range": "± 122434",
            "unit": "ns/iter"
          },
          {
            "name": "compression_levels/brotli_best",
            "value": 890930,
            "range": "± 6932",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_text_1kb",
            "value": 15629,
            "range": "± 988",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_json_1kb",
            "value": 17689,
            "range": "± 85",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_random_1kb",
            "value": 19199,
            "range": "± 108",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_text_100kb",
            "value": 29689,
            "range": "± 1627",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_json_100kb",
            "value": 29793,
            "range": "± 164",
            "unit": "ns/iter"
          },
          {
            "name": "data_types/zstd_random_100kb",
            "value": 30986,
            "range": "± 197",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/adaptive_tiny_text",
            "value": 392162,
            "range": "± 5067",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_zstd_tiny_text",
            "value": 15411,
            "range": "± 123",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_brotli_tiny_text",
            "value": 923849,
            "range": "± 162289",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/adaptive_small_json",
            "value": 2836111,
            "range": "± 17986",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_zstd_small_json",
            "value": 23734,
            "range": "± 133",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_brotli_small_json",
            "value": 1181518,
            "range": "± 48534",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/adaptive_large_text",
            "value": 8135800,
            "range": "± 71466",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_zstd_large_text",
            "value": 10284596,
            "range": "± 99905",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/adaptive_random",
            "value": 6537,
            "range": "± 99",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_zstd_random",
            "value": 18392,
            "range": "± 1069",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/adaptive_mixed_workload",
            "value": 3491646,
            "range": "± 33225",
            "unit": "ns/iter"
          },
          {
            "name": "adaptive_vs_static/static_zstd_mixed_workload",
            "value": 244202,
            "range": "± 840",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/balanced_text",
            "value": 954557,
            "range": "± 78375",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/balanced_json",
            "value": 952396,
            "range": "± 82761",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/speed_text",
            "value": 5775,
            "range": "± 26",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/speed_json",
            "value": 5731,
            "range": "± 33",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/max_compression_text",
            "value": 889431,
            "range": "± 4188",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/max_compression_json",
            "value": 1052043,
            "range": "± 17344",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/balanced_jpeg_store",
            "value": 295,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "per_type_compressor/balanced_git_blob",
            "value": 36817,
            "range": "± 265",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/baseline/1000",
            "value": 910,
            "range": "± 353",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/with_metrics/1000",
            "value": 5637,
            "range": "± 24",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/baseline/10000",
            "value": 8577,
            "range": "± 3609",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/with_metrics/10000",
            "value": 56656,
            "range": "± 557",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/baseline/100000",
            "value": 84727,
            "range": "± 36209",
            "unit": "ns/iter"
          },
          {
            "name": "dedup_overhead/with_metrics/100000",
            "value": 595932,
            "range": "± 24959",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/baseline/1000",
            "value": 1303,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/with_metrics/1000",
            "value": 50075,
            "range": "± 301",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/baseline/10000",
            "value": 12498,
            "range": "± 17",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/with_metrics/10000",
            "value": 500253,
            "range": "± 1209",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/baseline/100000",
            "value": 124463,
            "range": "± 324",
            "unit": "ns/iter"
          },
          {
            "name": "compression_overhead/with_metrics/100000",
            "value": 5002863,
            "range": "± 21041",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/baseline/1000",
            "value": 373,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/with_metrics/1000",
            "value": 4359,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/baseline/10000",
            "value": 3173,
            "range": "± 10",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/with_metrics/10000",
            "value": 43125,
            "range": "± 46",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/baseline/100000",
            "value": 31171,
            "range": "± 87",
            "unit": "ns/iter"
          },
          {
            "name": "cache_overhead/with_metrics/100000",
            "value": 431247,
            "range": "± 1723",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/baseline/1000",
            "value": 1457,
            "range": "± 2",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/with_metrics/1000",
            "value": 58911,
            "range": "± 148",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/baseline/10000",
            "value": 14083,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/with_metrics/10000",
            "value": 588537,
            "range": "± 5186",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/baseline/100000",
            "value": 140783,
            "range": "± 493",
            "unit": "ns/iter"
          },
          {
            "name": "operation_overhead/with_metrics/100000",
            "value": 5888453,
            "range": "± 21459",
            "unit": "ns/iter"
          },
          {
            "name": "comprehensive_mixed_metrics_100k",
            "value": 2231309,
            "range": "± 8518",
            "unit": "ns/iter"
          },
          {
            "name": "key_generation",
            "value": 57,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/64B",
            "value": 398,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/1KB",
            "value": 1287,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/16KB",
            "value": 20347,
            "range": "± 24",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/64KB",
            "value": 81132,
            "range": "± 124",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/128KB",
            "value": 120162,
            "range": "± 238",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/1MB",
            "value": 969434,
            "range": "± 3298",
            "unit": "ns/iter"
          },
          {
            "name": "encryption/encrypt/10MB",
            "value": 9756934,
            "range": "± 107927",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/64B",
            "value": 415,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/1KB",
            "value": 1271,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/16KB",
            "value": 14878,
            "range": "± 96",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/64KB",
            "value": 61198,
            "range": "± 121",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/128KB",
            "value": 121718,
            "range": "± 1096",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/1MB",
            "value": 966756,
            "range": "± 2050",
            "unit": "ns/iter"
          },
          {
            "name": "decryption/decrypt/10MB",
            "value": 9773175,
            "range": "± 88108",
            "unit": "ns/iter"
          },
          {
            "name": "round_trip/encrypt_decrypt/1KB",
            "value": 2592,
            "range": "± 21",
            "unit": "ns/iter"
          },
          {
            "name": "round_trip/encrypt_decrypt/64KB",
            "value": 142533,
            "range": "± 2296",
            "unit": "ns/iter"
          },
          {
            "name": "round_trip/encrypt_decrypt/1MB",
            "value": 1935760,
            "range": "± 34867",
            "unit": "ns/iter"
          },
          {
            "name": "argon2id_kdf/derive/testing_8MB_1iter",
            "value": 3366997,
            "range": "± 3228",
            "unit": "ns/iter"
          },
          {
            "name": "argon2id_kdf/derive/light_16MB_2iter",
            "value": 13816600,
            "range": "± 139444",
            "unit": "ns/iter"
          },
          {
            "name": "argon2id_kdf/derive/standard_32MB_3iter",
            "value": 47417252,
            "range": "± 550555",
            "unit": "ns/iter"
          },
          {
            "name": "stream_boundary/encrypt_below_64KB",
            "value": 60628,
            "range": "± 543",
            "unit": "ns/iter"
          },
          {
            "name": "stream_boundary/encrypt_at_64KB",
            "value": 60308,
            "range": "± 660",
            "unit": "ns/iter"
          },
          {
            "name": "stream_boundary/encrypt_above_64KB",
            "value": 60379,
            "range": "± 168",
            "unit": "ns/iter"
          },
          {
            "name": "encryption_overhead_small",
            "value": 463,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "encryption_overhead_large",
            "value": 241862,
            "range": "± 401",
            "unit": "ns/iter"
          },
          {
            "name": "cache_get/100",
            "value": 546,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "cache_get/1000",
            "value": 1126,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "cache_get/10000",
            "value": 6771,
            "range": "± 49",
            "unit": "ns/iter"
          },
          {
            "name": "cache_put/100",
            "value": 256,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "cache_put/1000",
            "value": 262,
            "range": "± 7",
            "unit": "ns/iter"
          },
          {
            "name": "cache_put/10000",
            "value": 284,
            "range": "± 9",
            "unit": "ns/iter"
          },
          {
            "name": "cache_eviction_100_entries",
            "value": 256,
            "range": "± 3",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/2",
            "value": 389233,
            "range": "± 18034",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/4",
            "value": 852599,
            "range": "± 39042",
            "unit": "ns/iter"
          },
          {
            "name": "concurrent_access/8",
            "value": 1631785,
            "range": "± 77944",
            "unit": "ns/iter"
          },
          {
            "name": "cache_stats",
            "value": 37,
            "range": "± 1",
            "unit": "ns/iter"
          },
          {
            "name": "branch_create",
            "value": 727629,
            "range": "± 115866",
            "unit": "ns/iter"
          },
          {
            "name": "branch_switch",
            "value": 639013,
            "range": "± 77357",
            "unit": "ns/iter"
          },
          {
            "name": "branch_list/list/10",
            "value": 431763,
            "range": "± 24726",
            "unit": "ns/iter"
          },
          {
            "name": "branch_list/list/50",
            "value": 1842151,
            "range": "± 87619",
            "unit": "ns/iter"
          },
          {
            "name": "branch_list/list/100",
            "value": 3731843,
            "range": "± 193184",
            "unit": "ns/iter"
          },
          {
            "name": "branch_delete",
            "value": 105505,
            "range": "± 5673",
            "unit": "ns/iter"
          },
          {
            "name": "branch_get_current",
            "value": 32097,
            "range": "± 2456",
            "unit": "ns/iter"
          },
          {
            "name": "branch_exists_check",
            "value": 26476,
            "range": "± 1845",
            "unit": "ns/iter"
          },
          {
            "name": "branch_update",
            "value": 754480,
            "range": "± 52390",
            "unit": "ns/iter"
          },
          {
            "name": "branch_concurrent/concurrent_read/2",
            "value": 193517,
            "range": "± 7217",
            "unit": "ns/iter"
          },
          {
            "name": "branch_concurrent/concurrent_read/4",
            "value": 174467,
            "range": "± 3347",
            "unit": "ns/iter"
          },
          {
            "name": "merge_fast_forward",
            "value": 3068,
            "range": "± 401",
            "unit": "ns/iter"
          },
          {
            "name": "merge_no_conflict/files/10",
            "value": 65324,
            "range": "± 13589",
            "unit": "ns/iter"
          },
          {
            "name": "merge_no_conflict/files/25",
            "value": 95962,
            "range": "± 16110",
            "unit": "ns/iter"
          },
          {
            "name": "merge_no_conflict/files/50",
            "value": 139840,
            "range": "± 19833",
            "unit": "ns/iter"
          },
          {
            "name": "merge_with_conflicts",
            "value": 24122,
            "range": "± 1805",
            "unit": "ns/iter"
          },
          {
            "name": "merge_strategies/strategy/Recursive",
            "value": 63257,
            "range": "± 4330",
            "unit": "ns/iter"
          },
          {
            "name": "merge_strategies/strategy/Ours",
            "value": 66145,
            "range": "± 5768",
            "unit": "ns/iter"
          },
          {
            "name": "merge_strategies/strategy/Theirs",
            "value": 66326,
            "range": "± 4832",
            "unit": "ns/iter"
          },
          {
            "name": "merge_lca_finding",
            "value": 62462,
            "range": "± 6021",
            "unit": "ns/iter"
          },
          {
            "name": "merge_tree_diff/changes/5",
            "value": 135922,
            "range": "± 10502",
            "unit": "ns/iter"
          },
          {
            "name": "merge_tree_diff/changes/10",
            "value": 134638,
            "range": "± 9711",
            "unit": "ns/iter"
          },
          {
            "name": "merge_tree_diff/changes/20",
            "value": 137690,
            "range": "± 10804",
            "unit": "ns/iter"
          },
          {
            "name": "odb_write/write/1KB",
            "value": 1046155,
            "range": "± 86400",
            "unit": "ns/iter"
          },
          {
            "name": "odb_write/write/10KB",
            "value": 1085006,
            "range": "± 73649",
            "unit": "ns/iter"
          },
          {
            "name": "odb_write/write/100KB",
            "value": 1479442,
            "range": "± 91780",
            "unit": "ns/iter"
          },
          {
            "name": "odb_write/write/1MB",
            "value": 4871763,
            "range": "± 290868",
            "unit": "ns/iter"
          },
          {
            "name": "odb_write/write/10MB",
            "value": 44242003,
            "range": "± 753868",
            "unit": "ns/iter"
          },
          {
            "name": "odb_read/read/1KB",
            "value": 244,
            "range": "± 12",
            "unit": "ns/iter"
          },
          {
            "name": "odb_read/read/10KB",
            "value": 438,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "odb_read/read/100KB",
            "value": 2537,
            "range": "± 8",
            "unit": "ns/iter"
          },
          {
            "name": "odb_read/read/1MB",
            "value": 28920,
            "range": "± 1856",
            "unit": "ns/iter"
          },
          {
            "name": "odb_cache_hit",
            "value": 230,
            "range": "± 4",
            "unit": "ns/iter"
          },
          {
            "name": "odb_dedup_check",
            "value": 36278,
            "range": "± 2579",
            "unit": "ns/iter"
          },
          {
            "name": "odb_exists_check",
            "value": 164,
            "range": "± 5",
            "unit": "ns/iter"
          },
          {
            "name": "odb_batch_write/batch/10",
            "value": 9257983,
            "range": "± 491129",
            "unit": "ns/iter"
          },
          {
            "name": "odb_batch_write/batch/100",
            "value": 88631007,
            "range": "± 8125511",
            "unit": "ns/iter"
          },
          {
            "name": "odb_concurrent/concurrent_read/2",
            "value": 41388,
            "range": "± 1875",
            "unit": "ns/iter"
          },
          {
            "name": "odb_concurrent/concurrent_read/4",
            "value": 76706,
            "range": "± 3332",
            "unit": "ns/iter"
          },
          {
            "name": "odb_concurrent/concurrent_read/8",
            "value": 122456,
            "range": "± 6476",
            "unit": "ns/iter"
          },
          {
            "name": "odb_metrics_collection",
            "value": 29,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "pack_write_single_1mb",
            "value": 752204,
            "range": "± 2666",
            "unit": "ns/iter"
          },
          {
            "name": "pack_write_many_100kb_objects",
            "value": 822824,
            "range": "± 1679",
            "unit": "ns/iter"
          },
          {
            "name": "pack_read_index_creation",
            "value": 688674,
            "range": "± 1896",
            "unit": "ns/iter"
          },
          {
            "name": "pack_read_single_object",
            "value": 60,
            "range": "± 0",
            "unit": "ns/iter"
          },
          {
            "name": "delta_encode_similar_10kb",
            "value": 49537080,
            "range": "± 550524",
            "unit": "ns/iter"
          },
          {
            "name": "delta_encode_very_similar_100kb",
            "value": 2138929329,
            "range": "± 7899873",
            "unit": "ns/iter"
          },
          {
            "name": "delta_encode_completely_different",
            "value": 371882,
            "range": "± 1176",
            "unit": "ns/iter"
          },
          {
            "name": "delta_apply_100kb",
            "value": 2118,
            "range": "± 6",
            "unit": "ns/iter"
          },
          {
            "name": "pack_write_with_deltas",
            "value": 2141485400,
            "range": "± 447188602",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}