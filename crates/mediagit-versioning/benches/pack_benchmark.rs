// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mediagit_versioning::{
    DeltaDecoder, DeltaEncoder, ObjectType, Oid, PackReader, PackWriter,
};

fn benchmark_pack_write(c: &mut Criterion) {
    c.bench_function("pack_write_single_1mb", |b| {
        b.iter(|| {
            let mut writer = PackWriter::new();
            let oid = Oid::hash(b"test");
            let data = black_box(vec![0u8; 1024 * 1024]); // 1MB

            writer.add_object(oid, ObjectType::Blob, &data);
            let _ = writer.finalize();
        });
    });

    c.bench_function("pack_write_many_100kb_objects", |b| {
        b.iter(|| {
            let mut writer = PackWriter::new();
            let data = black_box(vec![0u8; 100 * 1024]); // 100KB

            for i in 0u32..10 {
                let oid = Oid::hash(&i.to_le_bytes());
                writer.add_object(oid, ObjectType::Blob, &data);
            }

            let _ = writer.finalize();
        });
    });
}

fn benchmark_pack_read(c: &mut Criterion) {
    let mut writer = PackWriter::new();
    for i in 0u32..10 {
        let oid = Oid::hash(&i.to_le_bytes());
        let data = vec![i as u8; 100 * 1024]; // 100KB each
        writer.add_object(oid, ObjectType::Blob, &data);
    }
    let pack_data = writer.finalize();

    c.bench_function("pack_read_index_creation", |b| {
        b.iter(|| {
            let _ = PackReader::new(black_box(pack_data.clone()));
        });
    });

    let reader = PackReader::new(pack_data.clone()).unwrap();
    let oid = Oid::hash(&0u64.to_le_bytes());

    c.bench_function("pack_read_single_object", |b| {
        b.iter(|| {
            let _ = reader.get_object(&black_box(oid));
        });
    });
}

fn benchmark_delta_encoding(c: &mut Criterion) {
    c.bench_function("delta_encode_similar_10kb", |b| {
        b.iter_with_setup(
            || {
                let base = vec![0x42u8; 10 * 1024];
                let mut target = base.clone();
                target[5000] = 0x43;
                (base, target)
            },
            |(base, target)| {
                DeltaEncoder::encode(&base, &target)
            },
        );
    });

    c.bench_function("delta_encode_very_similar_100kb", |b| {
        b.iter_with_setup(
            || {
                let base = vec![0x42u8; 100 * 1024];
                let mut target = base.clone();
                for b in &mut target[50000..50100] {
                    *b = 0x43;
                }
                (base, target)
            },
            |(base, target)| {
                DeltaEncoder::encode(&base, &target)
            },
        );
    });

    c.bench_function("delta_encode_completely_different", |b| {
        b.iter_with_setup(
            || {
                let base = vec![0x00u8; 10 * 1024];
                let target = vec![0xFFu8; 10 * 1024];
                (base, target)
            },
            |(base, target)| {
                DeltaEncoder::encode(&base, &target)
            },
        );
    });
}

fn benchmark_delta_application(c: &mut Criterion) {
    let base = vec![0x42u8; 100 * 1024];
    let mut target = base.clone();
    for b in &mut target[50000..50100] {
        *b = 0x43;
    }
    let delta = DeltaEncoder::encode(&base, &target);

    c.bench_function("delta_apply_100kb", |b| {
        b.iter(|| {
            let _ = DeltaDecoder::apply(&base, &delta);
        });
    });
}

fn benchmark_pack_with_deltas(c: &mut Criterion) {
    c.bench_function("pack_write_with_deltas", |b| {
        b.iter_with_setup(
            || {
                let base_data = vec![0x42u8; 100 * 1024];
                let mut target = base_data.clone();
                for b in &mut target[50000..50100] {
                    *b = 0x43;
                }
                (base_data, target)
            },
            |(base_data, target)| {
                let mut writer = PackWriter::new();

                let base_oid = Oid::hash(b"base");
                writer.add_object(base_oid, ObjectType::Blob, &base_data);

                let delta = DeltaEncoder::encode(&base_data, &target);
                let target_oid = Oid::hash(b"target");
                writer.add_delta_object(target_oid, base_oid, &delta.to_bytes());

                writer.finalize()
            },
        );
    });
}

criterion_group!(
    benches,
    benchmark_pack_write,
    benchmark_pack_read,
    benchmark_delta_encoding,
    benchmark_delta_application,
    benchmark_pack_with_deltas,
);
criterion_main!(benches);
