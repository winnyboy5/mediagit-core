//! Encryption and KDF performance benchmarks
//!
//! Measures:
//! - AES-256-GCM encryption/decryption for various data sizes
//! - Stream encryption performance for large objects
//! - Argon2id key derivation with different parameters

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_security::{
    encryption::{decrypt, encrypt, EncryptionKey},
    kdf::{derive_key, Argon2Params, Salt},
};
use secrecy::SecretString;

/// Benchmark key generation
fn bench_key_generation(c: &mut Criterion) {
    c.bench_function("key_generation", |b| {
        b.iter(|| black_box(EncryptionKey::generate().unwrap()))
    });
}

/// Benchmark encryption for various data sizes
fn bench_encryption(c: &mut Criterion) {
    let key = EncryptionKey::generate().unwrap();

    let mut group = c.benchmark_group("encryption");

    // Test various data sizes from small to large
    let sizes: &[(usize, &str)] = &[
        (64, "64B"),
        (1024, "1KB"),
        (16 * 1024, "16KB"),
        (64 * 1024, "64KB"),          // Stream threshold
        (128 * 1024, "128KB"),        // Stream encryption
        (1024 * 1024, "1MB"),         // Typical image
        (10 * 1024 * 1024, "10MB"),   // Large media file
    ];

    for (size, label) in sizes {
        let data = vec![0xABu8; *size];

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::new("encrypt", label), &data, |b, data| {
            b.iter(|| black_box(encrypt(&key, black_box(data)).unwrap()))
        });
    }

    group.finish();
}

/// Benchmark decryption for various data sizes
fn bench_decryption(c: &mut Criterion) {
    let key = EncryptionKey::generate().unwrap();

    let mut group = c.benchmark_group("decryption");

    let sizes: &[(usize, &str)] = &[
        (64, "64B"),
        (1024, "1KB"),
        (16 * 1024, "16KB"),
        (64 * 1024, "64KB"),
        (128 * 1024, "128KB"),
        (1024 * 1024, "1MB"),
        (10 * 1024 * 1024, "10MB"),
    ];

    for (size, label) in sizes {
        let data = vec![0xABu8; *size];
        let ciphertext = encrypt(&key, &data).unwrap();

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(
            BenchmarkId::new("decrypt", label),
            &ciphertext,
            |b, ciphertext| b.iter(|| black_box(decrypt(&key, black_box(ciphertext)).unwrap())),
        );
    }

    group.finish();
}

/// Benchmark encrypt-decrypt round trip
fn bench_round_trip(c: &mut Criterion) {
    let key = EncryptionKey::generate().unwrap();

    let mut group = c.benchmark_group("round_trip");

    let sizes: &[(usize, &str)] = &[
        (1024, "1KB"),
        (64 * 1024, "64KB"),
        (1024 * 1024, "1MB"),
    ];

    for (size, label) in sizes {
        let data = vec![0xABu8; *size];

        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::new("encrypt_decrypt", label), &data, |b, data| {
            b.iter(|| {
                let ct = encrypt(&key, black_box(data)).unwrap();
                black_box(decrypt(&key, &ct).unwrap())
            })
        });
    }

    group.finish();
}

/// Benchmark Argon2id key derivation with different parameters
fn bench_kdf(c: &mut Criterion) {
    let password = SecretString::new("benchmark-password-12345".to_string());
    let salt = Salt::generate().unwrap();

    let mut group = c.benchmark_group("argon2id_kdf");
    group.sample_size(10); // KDF is slow, reduce sample size

    // Testing parameters (fast)
    let params_testing = Argon2Params::testing();
    group.bench_with_input(
        BenchmarkId::new("derive", "testing_8MB_1iter"),
        &(&password, &salt, params_testing),
        |b, (pw, s, p)| b.iter(|| black_box(derive_key(pw, s, *p).unwrap())),
    );

    // Light parameters (moderate security)
    let params_light = Argon2Params::custom(16384, 2, 2); // 16MB, 2 iterations, 2 threads
    group.bench_with_input(
        BenchmarkId::new("derive", "light_16MB_2iter"),
        &(&password, &salt, params_light),
        |b, (pw, s, p)| b.iter(|| black_box(derive_key(pw, s, *p).unwrap())),
    );

    // Standard parameters (production)
    let params_standard = Argon2Params::custom(32768, 3, 4); // 32MB, 3 iterations, 4 threads
    group.bench_with_input(
        BenchmarkId::new("derive", "standard_32MB_3iter"),
        &(&password, &salt, params_standard),
        |b, (pw, s, p)| b.iter(|| black_box(derive_key(pw, s, *p).unwrap())),
    );

    group.finish();
}

/// Benchmark stream encryption boundary behavior
fn bench_stream_boundary(c: &mut Criterion) {
    let key = EncryptionKey::generate().unwrap();

    let mut group = c.benchmark_group("stream_boundary");

    // Test just below and above the stream threshold (64KB)
    let below_threshold = vec![0xABu8; 64 * 1024 - 1]; // 64KB - 1
    let at_threshold = vec![0xABu8; 64 * 1024];        // 64KB exactly
    let above_threshold = vec![0xABu8; 64 * 1024 + 1]; // 64KB + 1

    group.throughput(Throughput::Bytes((64 * 1024 - 1) as u64));
    group.bench_function("encrypt_below_64KB", |b| {
        b.iter(|| black_box(encrypt(&key, black_box(&below_threshold)).unwrap()))
    });

    group.throughput(Throughput::Bytes((64 * 1024) as u64));
    group.bench_function("encrypt_at_64KB", |b| {
        b.iter(|| black_box(encrypt(&key, black_box(&at_threshold)).unwrap()))
    });

    group.throughput(Throughput::Bytes((64 * 1024 + 1) as u64));
    group.bench_function("encrypt_above_64KB", |b| {
        b.iter(|| black_box(encrypt(&key, black_box(&above_threshold)).unwrap()))
    });

    group.finish();
}

/// Benchmark encryption overhead analysis
fn bench_encryption_overhead(c: &mut Criterion) {
    let key = EncryptionKey::generate().unwrap();

    c.bench_function("encryption_overhead_small", |b| {
        let data = vec![0xABu8; 100];
        b.iter(|| {
            let ct = encrypt(&key, black_box(&data)).unwrap();
            // Overhead = ciphertext_len - plaintext_len
            let _overhead = ct.len() - data.len();
            black_box(ct)
        })
    });

    c.bench_function("encryption_overhead_large", |b| {
        let data = vec![0xABu8; 256 * 1024]; // 256KB (4 chunks)
        b.iter(|| {
            let ct = encrypt(&key, black_box(&data)).unwrap();
            let _overhead = ct.len() - data.len();
            black_box(ct)
        })
    });
}

criterion_group!(
    benches,
    bench_key_generation,
    bench_encryption,
    bench_decryption,
    bench_round_trip,
    bench_kdf,
    bench_stream_boundary,
    bench_encryption_overhead,
);
criterion_main!(benches);
