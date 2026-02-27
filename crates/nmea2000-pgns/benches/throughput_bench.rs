//! Throughput benchmark: measure how many messages/second we can decode.
//!
//! Uses canboat test vectors as the base corpus, repeated for volume.
//! Update test data with: `./scripts/update-canboat-tests.sh`
#![allow(clippy::unreadable_literal)]

use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use nmea2000_pgns::DecodedMessage;

const TEST_INPUT: &str = include_str!("../../../data/conformance/pgn-test.in");

/// Parse canboat test input format into (pgn, data) pairs.
fn parse_canboat_vectors() -> Vec<(u32, Vec<u8>)> {
    TEST_INPUT
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 7 {
                return None;
            }
            let pgn: u32 = parts[2].parse().ok()?;
            let data_len: usize = parts[5].parse().ok()?;
            let data: Vec<u8> = parts[6..]
                .iter()
                .take(data_len)
                .filter_map(|b| u8::from_str_radix(b.trim(), 16).ok())
                .collect();
            Some((pgn, data))
        })
        .collect()
}

fn bench_throughput_canboat_vectors(c: &mut Criterion) {
    let frames = parse_canboat_vectors();
    let count = frames.len() as u64;

    let mut group = c.benchmark_group("throughput");
    group.throughput(Throughput::Elements(count));

    group.bench_function(&format!("{count} canboat test vectors"), |b| {
        b.iter(|| {
            for (pgn, data) in black_box(&frames) {
                let _ = DecodedMessage::decode(*pgn, data);
            }
        });
    });
    group.finish();
}

fn bench_throughput_10k(c: &mut Criterion) {
    let base = parse_canboat_vectors();
    // Repeat to get ~10000 messages
    let repeat = 10000 / base.len().max(1);
    let frames: Vec<(u32, Vec<u8>)> = base
        .iter()
        .cycle()
        .take(repeat * base.len())
        .cloned()
        .collect();
    let count = frames.len() as u64;

    let mut group = c.benchmark_group("throughput");
    group.throughput(Throughput::Elements(count));

    group.bench_function(&format!("{count} mixed PGN decodes"), |b| {
        b.iter(|| {
            for (pgn, data) in black_box(&frames) {
                let _ = DecodedMessage::decode(*pgn, data);
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_throughput_canboat_vectors,
    bench_throughput_10k,
);
criterion_main!(benches);
