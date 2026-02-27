# nmea-2000-rs

[![CI](https://github.com/herostrat/nmea2000-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/herostrat/nmea2000-rs/actions/workflows/ci.yml)

High-performance NMEA 2000 protocol library for Rust, with 412+ auto-generated PGN types.

Built on the [canboat](https://github.com/canboat/canboat) project — the PGN definitions,
field types, lookup enumerations, and conformance test vectors are all derived from
canboat's open JSON database. This library is essentially a Rust-native implementation
of the protocol knowledge that canboat has accumulated over the years.

## Crates

| Crate | Description |
|-------|-------------|
| [`nmea2000`](crates/nmea2000/) | High-level bus interface with SocketCAN transport |
| [`nmea2000-pgns`](crates/nmea2000-pgns/) | 412+ generated PGN message types with typed field accessors |
| [`nmea2000-types`](crates/nmea2000-types/) | Core types: CAN ID parsing, Fast Packet & ISO TP reassembly, bit-level field extraction |
| [`nmea2000-codegen`](crates/nmea2000-codegen/) | Build-time code generator from canboat JSON database |

## Features

- **Type-safe PGN decoding/encoding** with auto-generated structs and field accessors
- **Multi-variant PGN dispatch** for PGNs with multiple definitions (126208, 126720, 130820, etc.)
- **Repeating field set iterators** for PGNs like GNSS Sats in View, PGN List
- **STRING_LAU/STRING_LZ parsing** with UTF-8 and UTF-16LE support
- **BCD/DECIMAL field decoding** (e.g., DSC Call Information)
- **Fast Packet reassembly** with out-of-order frame support (up to 64 concurrent sessions)
- **ISO Transport Protocol** (BAM and RTS/CTS) reassembly with automatic CTS/EOM_ACK responses
- **ISO TP BAM sending** for messages > 223 bytes
- **Bit-level field extraction** with optimized fast paths for aligned reads
- **SocketCAN integration** for Linux CAN bus interfaces

## Quick Start

```rust
use nmea2000_pgns::vessel_heading::VesselHeading;
use nmea2000_types::PgnMessage;

// Decode a Vessel Heading message from raw CAN data
let data = [0x00, 0x9F, 0x01, 0xFF, 0x7F, 0xFF, 0x7F, 0x00];
let msg = VesselHeading::decode(&data).unwrap();

assert_eq!(msg.sid(), Some(0));
assert_eq!(msg.heading(), Some(0.0415)); // radians

// Build and encode a message
let msg = VesselHeading::builder()
    .sid(42)
    .heading(1.5708) // ~90 degrees
    .build();

let mut buf = [0u8; 8];
msg.encode(&mut buf).unwrap();
```

## Hardware Setup

To use this library with a real NMEA 2000 bus, you need:

1. A USB-CAN adapter (e.g., Canable, Waveshare RS485/CAN HAT, Yacht Devices YDNU-02)
2. The adapter registers as a SocketCAN interface (`can0`) on Linux
3. Set up the interface: `sudo ip link set can0 up type can bitrate 250000`

```rust
use nmea2000::N2kBus;

let mut bus = N2kBus::open("can0").unwrap();
let msg = bus.read_message().unwrap();
println!("PGN: {}, Source: {}", msg.pgn.as_u32(), msg.source);
```

## Unit Conventions

This library returns values in SI base units. Your application handles conversions:

| Quantity | Library unit | Common display unit |
|----------|-------------|-------------------|
| Temperature | Kelvin | `K - 273.15` = Celsius |
| Pressure | Pascal | `Pa / 100000` = bar |
| Angles | Radians | `rad * 180/pi` = degrees |
| Angular rate | rad/s | `rad/s * 180/pi` = deg/s |

## Tests

The workspace has 105 tests across 8 test modules. Run them all with `cargo test --workspace`.

### Conformance (`nmea2000-pgns/tests/conformance.rs`) — 39 tests

Validates decoding and encoding against canboat's official test vectors. This is the
primary correctness check: real CAN bus captures are decoded and compared field-by-field
against known-good values.

- **Canboat vector tests** (`v01`–`v17`): Decode 17 real-world CAN captures (GNSS Position,
  ISO Address Claim, Engine Parameters, AIS, Heartbeat, etc.) and verify every field value.
- **Multi-variant dispatch** (`dispatch_pgn_126208_*`): PGN 126208 has 7 variants selected
  by a discriminator field — tests verify each function code routes to the correct struct.
- **Roundtrip tests** (`roundtrip_*`): Encode a message with the builder API, decode it back,
  and verify all fields match. Covers Vessel Heading, Wind Data, Battery Status, Water Depth,
  PGN List (repeating fields), and Configuration Information (STRING_LAU).
- **Unknown/sentinel handling**: Verifies that all-ones = unknown for unsigned fields,
  max-positive = unknown for signed fields, and 1-bit fields never have unknowns.
- **Enhanced Debug output**: Ensures `Debug` shows decoded field names and values,
  not raw byte arrays.
- **Error handling**: Insufficient data returns a proper error, unknown PGNs return the
  `Unknown` variant.

### Bus integration (`nmea2000/tests/mock_transport.rs`) — 12 tests

Tests the `N2kBus` read/write API with a mock CAN transport (in-memory frame queues).
Ensures all protocol layers work together end-to-end without real hardware.

- **Single frame**: Read and send single-frame PGN messages (<=8 bytes).
- **Fast Packet**: Fragment 20-byte messages into multiple CAN frames, send them,
  and reassemble on the other side. Includes a full send-then-receive roundtrip.
- **ISO TP BAM**: Send 300-byte payloads via Broadcast Announce Message (44 frames),
  verify frame structure, and roundtrip through reassembly.
- **ISO TP RTS/CTS**: Verify the bus sends CTS in response to RTS, and completes a full
  RTS → CTS → Data → EOM_ACK session. Verifies BAM sessions do *not* trigger CTS/ACK.
- **Boundary test**: 223 bytes uses Fast Packet, 224 bytes switches to ISO TP BAM.
- **PGN decoding**: Verifies `N2kBus::decode()` produces typed `DecodedMessage` from raw data.

### Fast Packet reassembly (`nmea2000-types/src/fast_packet.rs`) — 9 tests

Unit tests for the Fast Packet reassembler, which handles multi-frame PGN messages
up to 223 bytes with up to 64 concurrent sessions.

- Basic reassembly (20 bytes, 3 frames) and maximum size (223 bytes, 32 frames).
- **Out-of-order frames**: Frames arrive in scrambled order and still reassemble correctly.
- **Interleaved sessions**: Two different PGNs from the same source arrive interleaved.
- **Incomplete retry**: A partial transfer times out, then the same PGN is retransmitted successfully.
- **Buffer full**: All 64 session slots occupied returns a `BufferFull` error.
- **Canboat recombine scenarios** (`recombine_scenario1`–`5`): Replay frame sequences from
  canboat's `recombine-frames.in` test data.

### ISO TP reassembly (`nmea2000-types/src/iso_tp.rs`) — 7 tests

Unit tests for the ISO Transport Protocol manager (BAM and RTS/CTS modes) handling
messages up to 1785 bytes across up to 8 concurrent sessions.

- **BAM roundtrip**: Fragment and reassemble a 25-byte message via BAM.
- **RTS/CTS session setup**: Verifies `SendCts` action on RTS, `CompleteWithAck` on final data frame.
- **Out-of-order and duplicates**: BAM data frames arriving out of order or duplicated
  are handled correctly.
- **Retransmission**: An incomplete BAM session is superseded by a new announcement.
- **Large message**: Maximum-size ISO TP (1785 bytes, 255 packets).
- **Session slots exhausted**: All 8 slots occupied returns `BufferFull`.

### Bit-level field extraction (`nmea2000-types/src/field.rs`) — 24 tests

Unit tests for the low-level bitfield engine that reads/writes NMEA 2000 fields.

- **Byte-aligned extraction**: u8, u16, u32 at byte boundaries (fast path).
- **Unaligned extraction**: Arbitrary bit offsets and widths crossing byte boundaries.
- **Signed integers**: Sign extension for negative values, including 3-bit signed fields.
- **Insert/extract roundtrip**: Values survive a write-then-read cycle for both signed and unsigned.
- **Unknown detection**: All-ones sentinel detection for various bit widths (1–64 bits).
- **STRING_LAU**: UTF-8, UTF-16LE, and empty string parsing with length-and-encoding prefix.
- **STRING_LZ**: Null-terminated and unterminated string parsing.
- **BCD/DECIMAL**: Nibble-wise decimal decoding, unknown detection, and invalid nibble handling.

### CAN ID parsing (`nmea2000-types/src/id.rs`) — 7 tests

Tests for 29-bit extended CAN identifier encoding/decoding specific to NMEA 2000.

- Parse real CAN IDs for Vessel Heading (PDU2), ISO Request (PDU1), ISO Address Claim, and GNSS Position Data.
- Roundtrip tests for both PDU1 (addressed) and PDU2 (broadcast) identifier formats.
- Rejects identifiers that exceed 29 bits.

### PGN classification (`nmea2000-types/src/pgn.rs`) — 4 tests

Tests for PGN number interpretation: PDU1 vs PDU2 format, proprietary detection,
and packet type classification (Single, FastPacket, IsoTp).

### Codegen parser (`nmea2000-codegen/src/parser.rs`) — 3 tests

Tests that the canboat JSON database parses correctly at build time: verifies 100+ PGN
definitions are found, lookup enumerations (e.g., `MANUFACTURER_CODE`) are populated,
and individual PGNs contain their expected field definitions.

## Known Limitations

- **Address Claiming**: ISO 11783-5 Address Claim is not implemented. The library
  does not manage source addresses or participate in address negotiation. If you
  need this, please open an issue or submit a PR.
- **VARIABLE fields** (e.g., PGN 126208 Group Function): Iterators return only the
  fixed fields before the variable section. Access the raw bytes via `.raw()` for
  manual parsing of variable-length parameter data.
- **INDIRECT_LOOKUP fields**: Returned as raw numeric values. These fields require
  context from another field to resolve their meaning, which is not yet automated.
- **DYNAMIC_FIELD_\* fields**: Treated as raw binary data. These proprietary
  key-value fields vary by manufacturer.

## `no_std`

`no_std` is currently not implemented. The feature flag structure is prepared
(`nmea2000-types` and `nmea2000-pgns` have a `std` default feature), but
the actual `no_std` conversion has not been done yet. If you need `no_std`
support, please open an issue or submit a PR.

## License

Licensed under the [MIT License](LICENSE-MIT).
