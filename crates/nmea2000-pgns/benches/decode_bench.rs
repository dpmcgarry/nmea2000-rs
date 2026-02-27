//! PGN decode benchmarks using canboat test vectors.
//!
//! Test data sourced from canboat's `analyzer/tests/pgn-test.in`.
//! Update with: `./scripts/update-canboat-tests.sh`
#![allow(clippy::unreadable_literal)]

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use nmea2000_pgns::DecodedMessage;
use nmea2000_types::PgnMessage;

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

fn bench_decode_canboat_vectors(c: &mut Criterion) {
    let frames = parse_canboat_vectors();
    let count = frames.len();
    c.bench_function(&format!("decode {count} canboat test vectors"), |b| {
        b.iter(|| {
            for (pgn, data) in black_box(&frames) {
                let _ = DecodedMessage::decode(*pgn, data);
            }
        });
    });
}

fn bench_decode_single_pgns(c: &mut Criterion) {
    let frames = parse_canboat_vectors();
    let mut seen = std::collections::HashSet::new();

    // Bench one instance of each typed PGN from canboat test vectors
    for (pgn, data) in &frames {
        if !seen.insert(*pgn) {
            continue; // Skip duplicate PGNs
        }
        let name = match *pgn {
            60928 => "ISO Address Claim",
            126208 => "NMEA Group Function",
            126464 => "PGN List",
            126992 => "System Time",
            126993 => "Heartbeat",
            126998 => "Configuration Info",
            127251 => "Rate of Turn",
            127489 => "Engine Params Dynamic",
            127513 => "Battery Config",
            129029 => "GNSS Position Data",
            129039 => "AIS Class B",
            129540 => "GNSS Sats in View",
            129799 => "Radio Freq/Mode/Power",
            _ => "Unknown",
        };
        let label = format!("decode PGN {pgn} ({name})");
        let data = data.clone();
        c.bench_function(&label, |b| {
            b.iter(|| DecodedMessage::decode(*pgn, black_box(&data)));
        });
    }
}

fn bench_decode_and_access_fields(c: &mut Criterion) {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let data = [0x00, 0x9F, 0x01, 0xFF, 0x7F, 0xFF, 0x7F, 0xFD];
    c.bench_function("decode+access Vessel Heading (all fields)", |b| {
        b.iter(|| {
            let msg = VesselHeading::decode(black_box(&data)).unwrap();
            let _ = black_box(msg.sid());
            let _ = black_box(msg.heading());
            let _ = black_box(msg.deviation());
            let _ = black_box(msg.variation());
        });
    });
}

fn bench_decode_and_access_engine(c: &mut Criterion) {
    use nmea2000_pgns::engine_parameters_dynamic::EngineParametersDynamic;

    // Engine Params Dynamic — canboat test vector #6 (all fields populated)
    let data = [
        0x00, 0x2f, 0x06, 0x10, 0x20, 0xe3, 0x73, 0x65, 0x05, 0x65, 0x04, 0x72, 0x10, 0x00, 0x00,
        0x10, 0x20, 0x30, 0x40, 0xff, 0x06, 0x00, 0xff, 0x00, 0x30, 0x18,
    ];
    c.bench_function("decode+access Engine Params (13 fields)", |b| {
        b.iter(|| {
            let msg = EngineParametersDynamic::decode(black_box(&data)).unwrap();
            let _ = black_box(msg.instance_raw());
            let _ = black_box(msg.oil_pressure());
            let _ = black_box(msg.oil_temperature());
            let _ = black_box(msg.temperature());
            let _ = black_box(msg.alternator_potential());
            let _ = black_box(msg.fuel_rate());
            let _ = black_box(msg.total_engine_hours());
            let _ = black_box(msg.coolant_pressure());
            let _ = black_box(msg.fuel_pressure());
            let _ = black_box(msg.discrete_status1());
            let _ = black_box(msg.discrete_status2());
            let _ = black_box(msg.engine_load());
            let _ = black_box(msg.engine_torque());
        });
    });
}

fn bench_encode_vessel_heading(c: &mut Criterion) {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let msg = VesselHeading::builder().build();
    let mut buf = [0u8; 8];

    c.bench_function("encode PGN 127250 (Vessel Heading)", |b| {
        b.iter(|| msg.encode(black_box(&mut buf)));
    });
}

fn bench_decode_unknown_pgn(c: &mut Criterion) {
    let data = [0x00; 8];
    c.bench_function("decode unknown PGN (fallback)", |b| {
        b.iter(|| DecodedMessage::decode(999_999, black_box(&data)));
    });
}

criterion_group!(
    benches,
    bench_decode_canboat_vectors,
    bench_decode_single_pgns,
    bench_decode_and_access_fields,
    bench_decode_and_access_engine,
    bench_encode_vessel_heading,
    bench_decode_unknown_pgn,
);
criterion_main!(benches);
