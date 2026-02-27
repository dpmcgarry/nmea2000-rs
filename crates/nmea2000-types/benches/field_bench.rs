use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nmea2000_types::field::{extract_int, extract_uint, insert_uint};

fn bench_extract_aligned_u8(c: &mut Criterion) {
    let data = [0x42u8; 8];
    c.bench_function("extract_uint aligned u8", |b| {
        b.iter(|| extract_uint(black_box(&data), 0, 8));
    });
}

fn bench_extract_aligned_u16(c: &mut Criterion) {
    let data = [0x34, 0x12, 0x78, 0x56, 0x00, 0x00, 0x00, 0x00];
    c.bench_function("extract_uint aligned u16", |b| {
        b.iter(|| extract_uint(black_box(&data), 0, 16));
    });
}

fn bench_extract_aligned_u32(c: &mut Criterion) {
    let data = [0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00];
    c.bench_function("extract_uint aligned u32", |b| {
        b.iter(|| extract_uint(black_box(&data), 0, 32));
    });
}

fn bench_extract_unaligned(c: &mut Criterion) {
    let data = [0xFF; 8];
    c.bench_function("extract_uint unaligned 13-bit", |b| {
        b.iter(|| extract_uint(black_box(&data), 3, 13));
    });
}

fn bench_extract_signed(c: &mut Criterion) {
    let data = [0xFE, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    c.bench_function("extract_int signed i16", |b| {
        b.iter(|| extract_int(black_box(&data), 0, 16));
    });
}

fn bench_insert_aligned(c: &mut Criterion) {
    let mut data = [0u8; 8];
    c.bench_function("insert_uint aligned u16", |b| {
        b.iter(|| insert_uint(black_box(&mut data), 0, 16, 0x1234));
    });
}

fn bench_insert_unaligned(c: &mut Criterion) {
    let mut data = [0u8; 8];
    c.bench_function("insert_uint unaligned 13-bit", |b| {
        b.iter(|| insert_uint(black_box(&mut data), 3, 13, 0x1ABC));
    });
}

fn bench_extract_full_frame(c: &mut Criterion) {
    // Simulate extracting multiple fields from a typical 8-byte NMEA 2000 frame
    let data = [0x3F, 0x9F, 0x12, 0x7B, 0x05, 0xFA, 0xFF, 0xFF];
    c.bench_function("extract 5 fields from 8-byte frame", |b| {
        b.iter(|| {
            let _ = extract_uint(black_box(&data), 0, 8); // SID
            let _ = extract_uint(black_box(&data), 8, 16); // Heading
            let _ = extract_int(black_box(&data), 24, 16); // Deviation
            let _ = extract_int(black_box(&data), 40, 16); // Variation
            let _ = extract_uint(black_box(&data), 56, 2); // Reference
        });
    });
}

criterion_group!(
    benches,
    bench_extract_aligned_u8,
    bench_extract_aligned_u16,
    bench_extract_aligned_u32,
    bench_extract_unaligned,
    bench_extract_signed,
    bench_insert_aligned,
    bench_insert_unaligned,
    bench_extract_full_frame,
);
criterion_main!(benches);
