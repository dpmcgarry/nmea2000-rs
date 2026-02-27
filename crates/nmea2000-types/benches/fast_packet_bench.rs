#![allow(clippy::unreadable_literal)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nmea2000_types::fast_packet::{FastPacketFragmenter, FastPacketReassembler};
use nmea2000_types::pgn::Pgn;

fn bench_fragment_20_bytes(c: &mut Criterion) {
    let pgn = Pgn::new(129_029);
    let data: Vec<u8> = (0..20).collect();
    c.bench_function("fragment 20 bytes", |b| {
        b.iter(|| {
            let fragmenter = FastPacketFragmenter::new(pgn, 10, 3, 0, black_box(&data), 0);
            let _frames: Vec<_> = fragmenter.collect();
        });
    });
}

fn bench_fragment_223_bytes(c: &mut Criterion) {
    let pgn = Pgn::new(130_306);
    let data: Vec<u8> = (0..223).map(|i| (i & 0xFF) as u8).collect();
    c.bench_function("fragment 223 bytes (max)", |b| {
        b.iter(|| {
            let fragmenter = FastPacketFragmenter::new(pgn, 5, 2, 0, black_box(&data), 3);
            let _frames: Vec<_> = fragmenter.collect();
        });
    });
}

fn bench_reassemble_20_bytes(c: &mut Criterion) {
    let pgn = Pgn::new(129_029);
    let data: Vec<u8> = (0..20).collect();
    let frames: Vec<_> = FastPacketFragmenter::new(pgn, 10, 3, 0, &data, 0).collect();

    c.bench_function("reassemble 20 bytes", |b| {
        b.iter(|| {
            let mut reassembler = FastPacketReassembler::new();
            for (id, frame_data, len) in black_box(&frames) {
                let _ = reassembler.process_frame(*id, &frame_data[..*len]);
            }
        });
    });
}

fn bench_reassemble_223_bytes(c: &mut Criterion) {
    let pgn = Pgn::new(130_306);
    let data: Vec<u8> = (0..223).map(|i| (i & 0xFF) as u8).collect();
    let frames: Vec<_> = FastPacketFragmenter::new(pgn, 5, 2, 0, &data, 3).collect();

    c.bench_function("reassemble 223 bytes (max)", |b| {
        b.iter(|| {
            let mut reassembler = FastPacketReassembler::new();
            for (id, frame_data, len) in black_box(&frames) {
                let _ = reassembler.process_frame(*id, &frame_data[..*len]);
            }
        });
    });
}

fn bench_concurrent_reassembly(c: &mut Criterion) {
    let pgn_a = Pgn::new(129_029);
    let pgn_b = Pgn::new(130_306);
    let data_a: Vec<u8> = (0..50).collect();
    let data_b: Vec<u8> = (0..80).collect();
    let frames_a: Vec<_> = FastPacketFragmenter::new(pgn_a, 1, 3, 0, &data_a, 0).collect();
    let frames_b: Vec<_> = FastPacketFragmenter::new(pgn_b, 2, 3, 0, &data_b, 0).collect();

    // Interleave
    let mut interleaved = Vec::new();
    let max_len = frames_a.len().max(frames_b.len());
    for i in 0..max_len {
        if let Some(f) = frames_a.get(i) {
            interleaved.push(*f);
        }
        if let Some(f) = frames_b.get(i) {
            interleaved.push(*f);
        }
    }

    c.bench_function("reassemble 2 concurrent sessions", |b| {
        b.iter(|| {
            let mut reassembler = FastPacketReassembler::new();
            for (id, frame_data, len) in black_box(&interleaved) {
                let _ = reassembler.process_frame(*id, &frame_data[..*len]);
            }
        });
    });
}

criterion_group!(
    benches,
    bench_fragment_20_bytes,
    bench_fragment_223_bytes,
    bench_reassemble_20_bytes,
    bench_reassemble_223_bytes,
    bench_concurrent_reassembly,
);
criterion_main!(benches);
