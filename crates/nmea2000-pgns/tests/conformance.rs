//! Conformance tests against canboat's official test suite.
//!
//! Test vectors and expected values are from canboat's `analyzer/tests/`:
//! - Input:  `data/conformance/pgn-test.in`
//! - Output: `data/conformance/pgn-test-json-nv.out`
//!
//! Update with: `./scripts/update-canboat-tests.sh`
//!
//! ## Unit conversions
//!
//! Our library returns SI base units; canboat applies conversions:
//! - Pressure: our Pa → canboat bar (/100000)
//! - Temperature: our K → canboat °C (-273.15)
//! - Angles: our rad → canboat ° (*180/π)
//! - Angular rate: our rad/s → canboat °/s (*180/π)
//!
//! We verify at the SI level and note canboat's expected values in comments.
#![allow(clippy::unreadable_literal)]

use nmea2000_pgns::DecodedMessage;
use nmea2000_types::PgnMessage;

// ─── Test vector parsing ────────────────────────────────────────────────

const TEST_INPUT: &str = include_str!("../../../data/conformance/pgn-test.in");

struct TestVector {
    pgn: u32,
    source: u8,
    data: Vec<u8>,
}

fn parse_test_vectors() -> Vec<TestVector> {
    TEST_INPUT
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() < 7 {
                return None;
            }
            let pgn: u32 = parts[2].parse().ok()?;
            let source: u8 = parts[3].parse().ok()?;
            let data_len: usize = parts[5].parse().ok()?;
            let data: Vec<u8> = parts[6..]
                .iter()
                .take(data_len)
                .filter_map(|b| u8::from_str_radix(b.trim(), 16).ok())
                .collect();
            Some(TestVector { pgn, source, data })
        })
        .collect()
}

fn assert_near(actual: f64, expected: f64, tol: f64, field: &str) {
    assert!(
        (actual - expected).abs() < tol,
        "{field}: expected {expected}, got {actual} (diff={})",
        (actual - expected).abs()
    );
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 1: Full test-suite dispatch — every vector must decode
// ═══════════════════════════════════════════════════════════════════════

/// All 17 canboat test vectors must decode without error.
/// Typed PGNs must not fall through to the Unknown variant.
#[test]
fn all_canboat_vectors_decode() {
    let vectors = parse_test_vectors();
    assert_eq!(
        vectors.len(),
        17,
        "Expected 17 test vectors from pgn-test.in"
    );

    let expected_pgns = [
        129029, 127513, 60928, 60928, 127489, 127489, 129540, 126993, 126992, 126998, 126208,
        126208, 126208, 126464, 127251, 129039, 129799,
    ];

    for (i, (tv, &expected_pgn)) in vectors.iter().zip(expected_pgns.iter()).enumerate() {
        assert_eq!(tv.pgn, expected_pgn, "Vector {i}: PGN mismatch");
        let result = DecodedMessage::decode(tv.pgn, &tv.data);
        assert!(
            result.is_ok(),
            "Vector {i} (PGN {expected_pgn}): decode failed: {:?}",
            result.err()
        );
        assert!(
            !matches!(result.unwrap(), DecodedMessage::Unknown { .. }),
            "Vector {i} (PGN {expected_pgn}): should not be Unknown"
        );
    }
}

/// Unknown PGN returns Unknown variant (not an error).
#[test]
fn unknown_pgn_returns_unknown_variant() {
    let data = [0x01, 0x02, 0x03];
    match DecodedMessage::decode(999_999, &data).unwrap() {
        DecodedMessage::Unknown { pgn, data } => {
            assert_eq!(pgn.as_u32(), 999_999);
            assert_eq!(data, vec![1, 2, 3]);
        }
        _ => panic!("Expected Unknown variant"),
    }
}

/// All known PGNs can be decoded with zero/ones data without panicking.
#[test]
fn decode_smoke_test_no_panic() {
    let known_pgns: &[u32] = &[
        59904, 60928, 65240, 126208, 126464, 126992, 126993, 126996, 126998, 127250, 127251,
        127257, 127258, 127488, 127489, 127493, 127497, 127501, 127505, 127506, 127508, 127513,
        128259, 128267, 128275, 129025, 129026, 129029, 129033, 129038, 129039, 129040, 129041,
        129283, 129284, 129285, 129291, 129540, 129793, 129794, 129795, 129796, 129797, 129798,
        129799, 129800, 129801, 129802, 129808, 129809, 129810, 130306, 130310, 130311, 130312,
        130313, 130314, 130316,
    ];
    let zeros = [0u8; 223];
    let ones = [0xFFu8; 223];

    for pgn in known_pgns {
        let _ = DecodedMessage::decode(*pgn, &zeros);
        let _ = DecodedMessage::decode(*pgn, &ones);
        let _ = DecodedMessage::decode(*pgn, &zeros[..8]);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 2: Field-by-field verification against canboat reference
//
// Expected values from: data/conformance/pgn-test-json-nv.out
// For each PGN, canboat's JSON field values are noted in comments.
// Our library returns SI units; conversions are documented inline.
// ═══════════════════════════════════════════════════════════════════════

// ─── ISO Address Claim (PGN 60928) — vectors #3, #4 ─────────────────

/// Vector #3: Navico device (src=5)
/// canboat: Unique Number=1088507, Manufacturer Code=275 (Navico),
///   Device Instance Lower=0, Device Instance Upper=0,
///   Device Function=155 (Rudder), Device Class=40 (Steering),
///   System Instance=0, Industry Group=4 (Marine), Arbitrary=1 (Yes)
#[test]
fn v03_iso_address_claim_navico() {
    use nmea2000_pgns::iso_address_claim::IsoAddressClaim;
    let tv = &parse_test_vectors()[2];
    assert_eq!(tv.pgn, 60928);
    assert_eq!(tv.source, 5);

    let msg = IsoAddressClaim::decode(&tv.data).unwrap();
    assert_eq!(msg.unique_number(), Some(1088507));
    assert_eq!(msg.manufacturer_code_raw(), Some(275));
    assert_eq!(msg.device_instance_lower(), Some(0));
    assert_eq!(msg.device_instance_upper(), Some(0));
    assert_eq!(msg.device_function(), Some(155));
    assert_eq!(msg.device_class_raw(), Some(40));
    assert_eq!(msg.system_instance(), Some(0));
    assert_eq!(msg.industry_group_raw(), Some(4));
    assert_eq!(msg.arbitrary_address_capable_raw(), Some(1));
}

/// Vector #4: Airmar device (src=35)
/// canboat: Unique Number=321561, Manufacturer Code=135 (Airmar),
///   Device Function=130 (Bottom Depth), Device Class=60 (Navigation),
///   Industry Group=4 (Marine), Arbitrary=1 (Yes)
#[test]
fn v04_iso_address_claim_airmar() {
    use nmea2000_pgns::iso_address_claim::IsoAddressClaim;
    let tv = &parse_test_vectors()[3];
    assert_eq!(tv.pgn, 60928);
    assert_eq!(tv.source, 35);

    let msg = IsoAddressClaim::decode(&tv.data).unwrap();
    assert_eq!(msg.unique_number(), Some(321561));
    assert_eq!(msg.manufacturer_code_raw(), Some(135));
    assert_eq!(msg.device_instance_lower(), Some(0));
    assert_eq!(msg.device_instance_upper(), Some(0));
    assert_eq!(msg.device_function(), Some(130));
    assert_eq!(msg.device_class_raw(), Some(60));
    assert_eq!(msg.system_instance(), Some(0));
    assert_eq!(msg.industry_group_raw(), Some(4));
    assert_eq!(msg.arbitrary_address_capable_raw(), Some(1));
}

// ─── Battery Configuration Status (PGN 127513) — vector #2 ─────────

/// Vector #2: canboat: Instance=0, Battery Type=1 (Gel),
///   Supports Equalization=0 (No), Nominal Voltage=1 (12V),
///   Chemistry=1 (Li), Capacity=20, Temperature Coefficient=2,
///   Peukert Exponent=1.002, Charge Efficiency Factor=98
///
/// NOTE: Peukert Exponent discrepancy — canboat shows 1.002 (raw=1, *0.002 + 1.0 offset).
/// Our library returns 0.002 (no offset). Canboat's offset is hardcoded in their source.
#[test]
fn v02_battery_configuration_status() {
    use nmea2000_pgns::battery_configuration_status::BatteryConfigurationStatus;
    let tv = &parse_test_vectors()[1];
    assert_eq!(tv.pgn, 127513);

    let msg = BatteryConfigurationStatus::decode(&tv.data).unwrap();
    assert_eq!(msg.instance(), Some(0));
    assert_eq!(msg.battery_type_raw(), Some(1));
    assert_eq!(msg.supports_equalization_raw(), Some(0));
    assert_eq!(msg.nominal_voltage_raw(), Some(1));
    assert_eq!(msg.chemistry_raw(), Some(1));
    assert_eq!(msg.capacity(), Some(20));
    assert_eq!(msg.temperature_coefficient(), Some(2));
    // Peukert: raw=1, *0.002=0.002. Canboat adds implicit 1.0 offset → 1.002.
    assert_near(
        msg.peukert_exponent().unwrap(),
        1.002,
        0.001,
        "peukert_exponent",
    );
    assert_eq!(msg.charge_efficiency_factor(), Some(98));
}

// ─── Engine Parameters, Dynamic (PGN 127489) — vectors #5, #6 ──────

/// Vector #5: Some fields unknown.
/// canboat: Instance=0 (Single Engine), Oil pressure=1.583 bar,
///   Temperature=23.52°C, Alternator Potential=13.81V,
///   Total Engine hours=4210s (01:10:10),
///   Discrete Status 1=[Over Temperature(2), Low Oil Pressure(4)]
#[test]
fn v05_engine_parameters_dynamic_partial() {
    use nmea2000_pgns::engine_parameters_dynamic::EngineParametersDynamic;
    let tv = &parse_test_vectors()[4];
    assert_eq!(tv.pgn, 127489);

    let msg = EngineParametersDynamic::decode(&tv.data).unwrap();
    assert_eq!(msg.instance_raw(), Some(0));
    // Oil pressure: canboat 1.583 bar = 158300 Pa
    assert_near(msg.oil_pressure().unwrap(), 158300.0, 1.0, "oil_pressure");
    // Oil temperature: unknown (0xFFFF)
    assert!(msg.oil_temperature().is_none());
    // Temperature: canboat 23.52°C = 296.67 K
    assert_near(msg.temperature().unwrap(), 296.67, 0.01, "temperature");
    // Alternator Potential: 13.81 V (same unit)
    assert_near(
        msg.alternator_potential().unwrap(),
        13.81,
        0.01,
        "alternator_potential",
    );
    // Fuel Rate: signed unknown (0x7FFF)
    assert!(msg.fuel_rate().is_none());
    // Total Engine hours: canboat raw 4210 s
    assert_eq!(msg.total_engine_hours(), Some(4210));
    // Coolant Pressure: unknown (0xFFFF)
    assert!(msg.coolant_pressure().is_none());
    // Fuel Pressure: unknown (0xFFFF)
    assert!(msg.fuel_pressure().is_none());
    // Discrete Status 1: 0x0006 = bits 1+2 (Over Temperature + Low Oil Pressure)
    assert_eq!(msg.discrete_status1(), Some(6));
    // Discrete Status 2: 0x0000
    assert_eq!(msg.discrete_status2(), Some(0));
    // Engine Load: signed unknown (0x7F)
    assert!(msg.engine_load().is_none());
    // Engine Torque: signed unknown (0x7F)
    assert!(msg.engine_torque().is_none());
}

/// Vector #6: All fields populated.
/// canboat: Oil pressure=1.583, Oil temperature=547.7°C,
///   Temperature=23.52°C, Alternator Potential=13.81V, Fuel Rate=112.5 L/h,
///   Total Engine hours=4210s, Coolant Pressure=8.208 bar,
///   Fuel Pressure=164.320 bar, Discrete Status 2=[all 8 flags],
///   Engine Load=48%, Engine Torque=24%
#[test]
fn v06_engine_parameters_dynamic_full() {
    use nmea2000_pgns::engine_parameters_dynamic::EngineParametersDynamic;
    let tv = &parse_test_vectors()[5];
    assert_eq!(tv.pgn, 127489);

    let msg = EngineParametersDynamic::decode(&tv.data).unwrap();
    assert_eq!(msg.instance_raw(), Some(0));
    assert_near(msg.oil_pressure().unwrap(), 158300.0, 1.0, "oil_pressure");
    // Oil temperature: canboat 547.7°C = 820.85 K
    assert_near(
        msg.oil_temperature().unwrap(),
        820.8,
        0.1,
        "oil_temperature",
    );
    assert_near(msg.temperature().unwrap(), 296.67, 0.01, "temperature");
    assert_near(
        msg.alternator_potential().unwrap(),
        13.81,
        0.01,
        "alternator_potential",
    );
    // Fuel Rate: 112.5 L/h (same unit)
    assert_near(msg.fuel_rate().unwrap(), 112.5, 0.1, "fuel_rate");
    assert_eq!(msg.total_engine_hours(), Some(4210));
    // Coolant Pressure: canboat 8.208 bar = 820800 Pa
    assert_near(
        msg.coolant_pressure().unwrap(),
        820800.0,
        1.0,
        "coolant_pressure",
    );
    // Fuel Pressure: canboat 164.320 bar = 16432000 Pa
    assert_near(
        msg.fuel_pressure().unwrap(),
        16432000.0,
        1.0,
        "fuel_pressure",
    );
    assert_eq!(msg.discrete_status1(), Some(6));
    // Discrete Status 2: 0x00FF = all 8 lower flags
    assert_eq!(msg.discrete_status2(), Some(255));
    assert_eq!(msg.engine_load(), Some(48));
    assert_eq!(msg.engine_torque(), Some(24));
}

// ─── Heartbeat (PGN 126993) — vector #8 ────────────────────────────

/// Vector #8: canboat: Data transmit offset raw=1 (00:00:00.01),
///   Sequence Counter=36
#[test]
fn v08_heartbeat() {
    use nmea2000_pgns::heartbeat::Heartbeat;
    let tv = &parse_test_vectors()[7];
    assert_eq!(tv.pgn, 126993);

    let msg = Heartbeat::decode(&tv.data).unwrap();
    // Data transmit offset: raw 1 * 0.01 = 0.01 s
    assert_near(
        msg.data_transmit_offset().unwrap(),
        0.01,
        0.001,
        "data_transmit_offset",
    );
    assert_eq!(msg.sequence_counter(), Some(36));
}

// ─── System Time (PGN 126992) — vector #9 ──────────────────────────

/// Vector #9: canboat: SID=16, Source=0 (GPS),
///   Date raw=15089 (2011.04.25), Time raw=370100001 (10:16:50.0001)
#[test]
fn v09_system_time() {
    use nmea2000_pgns::system_time::SystemTime;
    let tv = &parse_test_vectors()[8];
    assert_eq!(tv.pgn, 126992);

    let msg = SystemTime::decode(&tv.data).unwrap();
    assert_eq!(msg.sid(), Some(16));
    assert_eq!(msg.source_raw(), Some(0)); // GPS
    assert_eq!(msg.date(), Some(15089));
    // Time: raw 370100001 * 0.0001 = 37010.0000 s
    assert_near(msg.time().unwrap(), 37010.0000, 0.001, "time");
}

// ─── Rate of Turn (PGN 127251) — vector #15 ────────────────────────

/// Vector #15: canboat: Rate=-0.029649 (deg/s)
/// Our lib returns rad/s. Raw i32 = 0xFFFFbf51 = -16559, *3.125e-8 rad/s
#[test]
fn v15_rate_of_turn() {
    use nmea2000_pgns::rate_of_turn::RateOfTurn;
    let tv = &parse_test_vectors()[14];
    assert_eq!(tv.pgn, 127251);

    let msg = RateOfTurn::decode(&tv.data).unwrap();
    // SID: 0xFF = unknown
    assert!(msg.sid().is_none());
    let rate_rad_s = msg.rate().unwrap();
    let expected_rad_s = -16559.0 * 3.125e-8;
    assert_near(rate_rad_s, expected_rad_s, 1e-9, "rate (rad/s)");
    // Verify canboat's converted value
    let rate_deg_s = rate_rad_s * (180.0 / std::f64::consts::PI);
    assert_near(rate_deg_s, -0.029649, 0.0001, "rate (deg/s, canboat)");
}

// ─── AIS Class B Position Report (PGN 129039) — vector #16 ─────────

/// Vector #16: canboat: Message ID=18, Repeat=0, User ID=244180106,
///   Longitude=5.3134516, Latitude=52.9061666, Position Accuracy=1 (High),
///   RAIM=1 (in use), Time Stamp=29, COG=171.7°, SOG=1.80 m/s,
///   AIS Transceiver=0 (Channel A VDL), Unit type=0 (SOTDMA),
///   Integrated Display=0 (No), DSC=1 (Yes), Band=1 (Entire),
///   Can handle Msg 22=1 (Yes), AIS mode=1 (Assigned),
///   AIS communication state=0 (SOTDMA)
#[test]
fn v16_ais_class_b_position_report() {
    use nmea2000_pgns::ais_class_b_position_report::AisClassBPositionReport;
    let tv = &parse_test_vectors()[15];
    assert_eq!(tv.pgn, 129039);

    let msg = AisClassBPositionReport::decode(&tv.data).unwrap();
    assert_eq!(msg.message_id_raw(), Some(18));
    assert_eq!(msg.repeat_indicator_raw(), Some(0));
    assert_eq!(msg.user_id(), Some(244180106));
    // Longitude/Latitude: our lib returns degrees (resolution 1e-7 deg)
    assert_near(msg.longitude().unwrap(), 5.3134516, 1e-6, "longitude");
    assert_near(msg.latitude().unwrap(), 52.9061666, 1e-6, "latitude");
    assert_eq!(msg.position_accuracy_raw(), Some(1));
    assert_eq!(msg.raim_raw(), Some(1));
    // Time Stamp: 6-bit value=29 (seconds). Not in LOOKUP enum (only 60-63 defined).
    assert_eq!(msg.time_stamp_raw(), Some(29));
    // COG: canboat 171.7° → our rad
    let cog_deg = msg.cog().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(cog_deg, 171.7, 0.1, "COG (deg, canboat)");
    // SOG: 1.80 m/s (same unit)
    assert_near(msg.sog().unwrap(), 1.80, 0.01, "SOG");
    assert_eq!(msg.ais_transceiver_information_raw(), Some(0));
    assert_eq!(msg.unit_type_raw(), Some(0));
    // Additional fields from canboat reference
    assert_eq!(msg.integrated_display_raw(), Some(0));
    assert_eq!(msg.dsc_raw(), Some(1));
    assert_eq!(msg.band_raw(), Some(1));
    assert_eq!(msg.can_handle_msg22_raw(), Some(1));
    assert_eq!(msg.ais_mode_raw(), Some(1));
    assert_eq!(msg.ais_communication_state_raw(), Some(0));
}

// ─── Radio Frequency/Mode/Power (PGN 129799) — vector #17 ──────────

/// Vector #17: canboat: Rx Frequency=156800000, Tx Frequency=156800000,
///   Radio Channel="900016", Tx Power=25, Mode=0 (F3E/G3E simplex),
///   Channel Bandwidth=25000
#[test]
fn v17_radio_frequency_mode_power() {
    use nmea2000_pgns::radio_frequency_mode_power::RadioFrequencyModePower;
    let tv = &parse_test_vectors()[16];
    assert_eq!(tv.pgn, 129799);

    let msg = RadioFrequencyModePower::decode(&tv.data).unwrap();
    // Rx Frequency: raw 15680000 * 10 = 156800000 Hz
    assert_near(
        msg.rx_frequency().unwrap(),
        156800000.0,
        1.0,
        "rx_frequency",
    );
    assert_near(
        msg.tx_frequency().unwrap(),
        156800000.0,
        1.0,
        "tx_frequency",
    );
    assert_eq!(msg.radio_channel(), Some("900016"));
    assert_eq!(msg.tx_power(), Some(25));
    assert_eq!(msg.mode_raw(), Some(0));
    assert_eq!(msg.channel_bandwidth(), Some(25000));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 3: Variable-length / complex PGNs — verify basic field access
// These PGNs have repeating fields or complex structures.
// We verify dispatch works and basic fields where possible.
// ═══════════════════════════════════════════════════════════════════════

/// Vector #1: GNSS Position Data (PGN 129029) — 43 bytes, has repeating fields.
/// canboat: SID=230, Date=15089, Time=231120000 (06:25:12.0000),
///   Latitude=52.7461333, Longitude=5.1815566,
///   Altitude=3.400000, GNSS type=3 (GPS+SBAS), Method=1 (fix), Integrity=0,
///   Number of SVs=9, HDOP=0.90, PDOP=1.40,
///   Geoidal Separation=47.04, Reference Stations=0
#[test]
fn v01_gnss_position_data() {
    use nmea2000_pgns::gnss_position_data::GnssPositionData;
    let tv = &parse_test_vectors()[0];
    assert_eq!(tv.pgn, 129029);

    let msg = GnssPositionData::decode(&tv.data).unwrap();
    assert_eq!(msg.sid(), Some(230));
    assert_eq!(msg.date(), Some(15089u16));
    // Time: raw 231120000 * 0.0001 = 23112.0 seconds (06:25:12.0000)
    assert_near(msg.time().unwrap(), 23112.0, 0.01, "time");
    assert_near(msg.latitude().unwrap(), 52.7461333, 1e-6, "latitude");
    assert_near(msg.longitude().unwrap(), 5.1815566, 1e-6, "longitude");
    assert_near(msg.altitude().unwrap(), 3.4, 0.001, "altitude");
    assert_eq!(msg.gnss_type_raw(), Some(3)); // GPS+SBAS/WAAS
    assert_eq!(msg.method_raw(), Some(1)); // GNSS fix
    assert_eq!(msg.integrity_raw(), Some(0)); // No integrity checking
    assert_eq!(msg.number_of_svs(), Some(9));
    assert_near(msg.hdop().unwrap(), 0.90, 0.01, "hdop");
    assert_near(msg.pdop().unwrap(), 1.40, 0.01, "pdop");
    // Geoidal Separation: not in canboat output → unknown sentinel
    assert!(msg.geoidal_separation().is_none());
    assert_eq!(msg.reference_stations(), Some(0));
    // Repeating set: 0 reference stations → empty iterator
    assert_eq!(msg.list().count(), 0);
}

/// Vector #7: GNSS Sats in View (PGN 129540) — 216 bytes, 18 satellite entries.
/// canboat: SID=4, Sats in View=18, 18 satellite entries.
/// canboat warns: "PGN 129540 has 2 missing fields in repeating set"
/// (last satellite truncated — missing Status field).
///
/// First satellite: PRN=3, Elevation=52.0°, Azimuth=88.0°, SNR=33.00, Status=2 (Used)
#[test]
fn v07_gnss_sats_in_view() {
    use nmea2000_pgns::gnss_sats_in_view::GnssSatsInView;
    let tv = &parse_test_vectors()[6];
    assert_eq!(tv.pgn, 129540);

    let msg = GnssSatsInView::decode(&tv.data).unwrap();
    assert_eq!(msg.sid(), Some(4));
    assert_eq!(msg.sats_in_view(), Some(18));

    let entries: Vec<_> = msg.list().collect();
    // canboat reports 18 sats but warns "2 missing fields in repeating set" for
    // the last entry (PRN=74, truncated). Our iterator skips incomplete entries.
    assert_eq!(entries.len(), 17);

    // Satellite 1: PRN=3, Elevation=52.0°, Azimuth=88.0°, SNR=33.00 dB
    let sat = &entries[0];
    assert_eq!(sat.prn(), Some(3));
    // Elevation/Azimuth: canboat shows degrees, our lib returns radians
    let elev_deg = sat.elevation().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(elev_deg, 52.0, 0.1, "sat1 elevation");
    let azim_deg = sat.azimuth().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(azim_deg, 88.0, 0.1, "sat1 azimuth");
    assert_near(sat.snr().unwrap(), 33.00, 0.01, "sat1 snr");
    assert_near(
        sat.range_residuals().unwrap(),
        0.0,
        0.001,
        "sat1 range_residuals",
    );
    assert_eq!(sat.status_raw(), Some(2)); // Used

    // Satellite 10: PRN=9, Elevation=46.0°, Azimuth=210.0°, SNR=29.00 dB
    let sat10 = &entries[9];
    assert_eq!(sat10.prn(), Some(9));
    let elev10_deg = sat10.elevation().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(elev10_deg, 46.0, 0.1, "sat10 elevation");
    let azim10_deg = sat10.azimuth().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(azim10_deg, 210.0, 0.1, "sat10 azimuth");
    assert_near(sat10.snr().unwrap(), 29.00, 0.01, "sat10 snr");
    assert_eq!(sat10.status_raw(), Some(2)); // Used

    // Satellite 17 (PRN=25): last complete entry
    let sat17 = &entries[16];
    assert_eq!(sat17.prn(), Some(25));
    let elev17_deg = sat17.elevation().unwrap() * (180.0 / std::f64::consts::PI);
    assert_near(elev17_deg, 4.0, 0.1, "sat17 elevation");
    assert_eq!(sat17.status_raw(), Some(2)); // Used
}

/// Vector #10: Configuration Information (PGN 126998) — STRING_LZ/STRING_LAU.
/// canboat: Installation Description #1="hello", #2="wórld" (UTF-16LE)
#[test]
fn v10_configuration_information_string_lau() {
    use nmea2000_pgns::configuration_information::ConfigurationInformation;

    let tv = &parse_test_vectors()[9];
    assert_eq!(tv.pgn, 126998);

    // Verify dispatch
    let msg = DecodedMessage::decode(tv.pgn, &tv.data).unwrap();
    assert!(!matches!(msg, DecodedMessage::Unknown { .. }));

    // Verify STRING_LAU field parsing
    // canboat: Installation Description #1 = "hello" (UTF-8, enc=0x01)
    // canboat: Installation Description #2 = "wórld" (UTF-16LE, enc=0x00)
    let msg = ConfigurationInformation::decode(&tv.data).unwrap();
    assert_eq!(msg.installation_description1(), Some("hello".to_string()));
    assert_eq!(msg.installation_description2(), Some("wórld".to_string()));
}

/// Vector #11: NMEA - Command group function (PGN 126208, function_code=1)
/// canboat: Function Code=1 (Command), PGN=126998, Number of Parameters=1
#[test]
fn v11_nmea_command_group_function() {
    use nmea2000_pgns::nmea_command_group_function::NmeaCommandGroupFunction;
    let tv = &parse_test_vectors()[10];
    assert_eq!(tv.pgn, 126208);

    let msg = NmeaCommandGroupFunction::decode(&tv.data).unwrap();
    assert_eq!(msg.function_code_raw(), Some(1)); // Command
    assert_eq!(msg.pgn(), Some(126998)); // Configuration Information
    assert_eq!(msg.number_of_parameters(), Some(1));
}

/// Vector #12: NMEA - Acknowledge group function (PGN 126208, function_code=2)
/// canboat: Function Code=2 (Acknowledge), PGN=65410,
///   PGN error code=0 (Acknowledge),
///   Transmission interval/Priority error code=1 (not supported),
///   Number of Parameters=2
#[test]
fn v12_nmea_acknowledge_group_function() {
    use nmea2000_pgns::nmea_acknowledge_group_function::NmeaAcknowledgeGroupFunction;
    let tv = &parse_test_vectors()[11];
    assert_eq!(tv.pgn, 126208);

    let msg = NmeaAcknowledgeGroupFunction::decode(&tv.data).unwrap();
    assert_eq!(msg.function_code_raw(), Some(2)); // Acknowledge
    assert_eq!(msg.pgn(), Some(65410));
    assert_eq!(msg.pgn_error_code_raw(), Some(0)); // Acknowledge
    assert_eq!(
        msg.transmission_interval_priority_error_code_raw(),
        Some(1) // Not supported
    );
    assert_eq!(msg.number_of_parameters(), Some(2));
    // Repeating set: 2 parameter entries, both value=0 (Acknowledge)
    let entries: Vec<_> = msg.list().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].parameter_raw(), Some(0));
    assert_eq!(entries[1].parameter_raw(), Some(0));
}

/// Vector #13: NMEA - Read Fields group function (PGN 126208, function_code=3)
/// canboat: Function Code=3 (Read Fields), PGN=130306 (Wind Data),
///   Unique ID=0, Number of Selection Pairs=1, Number of Parameters=2
///
/// NOTE: Generated offsets assume all conditional fields (manufacturer_code,
/// industry_code) are present. This test vector uses a non-proprietary PGN
/// (130306), so those fields are absent and post-conditional field offsets
/// are shifted. Only function_code and pgn are verified here.
#[test]
fn v13_nmea_read_fields_group_function() {
    use nmea2000_pgns::nmea_read_fields_group_function::NmeaReadFieldsGroupFunction;
    let tv = &parse_test_vectors()[12];
    assert_eq!(tv.pgn, 126208);

    let msg = NmeaReadFieldsGroupFunction::decode(&tv.data).unwrap();
    assert_eq!(msg.function_code_raw(), Some(3)); // Read Fields
    assert_eq!(msg.pgn(), Some(130306)); // Wind Data
}

/// Vector #14: PGN List (PGN 126464) — repeating PGN entries.
/// canboat: Function Code=1 (Receive), list=[130820, 129809]
#[test]
fn v14_pgn_list() {
    use nmea2000_pgns::pgn_list_transmit_and_receive::PgnListTransmitAndReceive;
    let tv = &parse_test_vectors()[13];
    assert_eq!(tv.pgn, 126464);

    let msg = PgnListTransmitAndReceive::decode(&tv.data).unwrap();
    assert_eq!(msg.function_code_raw(), Some(1)); // Receive PGN list
    // Repeating set: 2 PGN entries
    let entries: Vec<_> = msg.list().collect();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].pgn(), Some(130820));
    assert_eq!(entries[1].pgn(), Some(129809));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 4: Dynamic PGN builder roundtrip tests
// ═══════════════════════════════════════════════════════════════════════

/// Roundtrip: PGN List (126464) — build with 3 PGN entries, verify via iterator.
#[test]
fn roundtrip_pgn_list_builder() {
    use nmea2000_pgns::pgn_list_transmit_and_receive::PgnListTransmitAndReceive;
    use nmea2000_types::PgnMessage;

    let msg = PgnListTransmitAndReceive::builder()
        .function_code_raw(1) // Transmit
        .add_entry(130820)
        .add_entry(129029)
        .add_entry(126992)
        .build();

    // Verify decode
    assert_eq!(msg.function_code_raw(), Some(1));
    let entries: Vec<_> = msg.list().collect();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].pgn(), Some(130820));
    assert_eq!(entries[1].pgn(), Some(129029));
    assert_eq!(entries[2].pgn(), Some(126992));

    // Verify encode roundtrip
    let mut buf = vec![0u8; msg.data_length()];
    let len = msg.encode(&mut buf).unwrap();
    let decoded = PgnListTransmitAndReceive::decode(&buf[..len]).unwrap();
    assert_eq!(decoded.function_code_raw(), Some(1));
    let decoded_entries: Vec<_> = decoded.list().collect();
    assert_eq!(decoded_entries.len(), 3);
    assert_eq!(decoded_entries[0].pgn(), Some(130820));
}

/// Roundtrip: Configuration Information (126998) — STRING_LAU fields.
#[test]
fn roundtrip_configuration_information_builder() {
    use nmea2000_pgns::configuration_information::ConfigurationInformation;
    use nmea2000_types::PgnMessage;

    let msg = ConfigurationInformation::builder()
        .installation_description1("Engine Room")
        .installation_description2("Port Side")
        .manufacturer_information("Acme Marine v2.1")
        .build();

    // Verify encode roundtrip
    let mut buf = vec![0u8; msg.data_length()];
    let len = msg.encode(&mut buf).unwrap();
    let decoded = ConfigurationInformation::decode(&buf[..len]).unwrap();

    // The raw bytes should survive the roundtrip
    assert_eq!(msg.raw(), decoded.raw());
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 5: Unknown sentinel detection
// These tests verify correct handling of null/unknown values in NMEA 2000.
// ═══════════════════════════════════════════════════════════════════════

/// Unsigned fields: all-ones (0xFF...) = unknown sentinel.
#[test]
fn unsigned_fields_all_ones_are_unknown() {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let data = [0xFF; 8];
    let msg = VesselHeading::decode(&data).unwrap();

    assert!(msg.sid().is_none(), "SID (u8) 0xFF = unknown");
    assert!(msg.heading().is_none(), "Heading (u16) 0xFFFF = unknown");
    // Deviation/Variation are signed: 0xFFFF = -1 (valid, not unknown)
    assert!(msg.deviation().is_some(), "Deviation (i16) -1 is valid");
    assert!(msg.variation().is_some(), "Variation (i16) -1 is valid");
}

/// Signed fields: max positive value = unknown sentinel.
/// For i16: 0x7FFF = 32767. For i8: 0x7F = 127.
#[test]
fn signed_fields_max_positive_is_unknown() {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let mut data = [0x00; 8];
    data[3] = 0xFF; // Deviation lower byte
    data[4] = 0x7F; // Deviation upper byte → i16 = 0x7FFF
    let msg = VesselHeading::decode(&data).unwrap();
    assert!(
        msg.deviation().is_none(),
        "Deviation 0x7FFF = signed unknown"
    );
}

/// 1-bit fields have no unknown sentinel (no room for it).
#[test]
fn one_bit_fields_never_unknown() {
    use nmea2000_pgns::iso_address_claim::IsoAddressClaim;

    // arbitrary_address_capable is 1-bit at bit 63
    // When set to 1, it must return Some(1), not None
    let data = [0xfb, 0x9b, 0x70, 0x22, 0x00, 0x9b, 0x50, 0xc0];
    let msg = IsoAddressClaim::decode(&data).unwrap();
    assert_eq!(msg.arbitrary_address_capable_raw(), Some(1));
}

/// Engine params: mixed unsigned/signed unknown detection.
#[test]
fn engine_params_all_ones_mixed_unknown() {
    use nmea2000_pgns::engine_parameters_dynamic::EngineParametersDynamic;

    let data = [0xFF; 26];
    let msg = EngineParametersDynamic::decode(&data).unwrap();

    // Unsigned: all-ones = unknown
    assert!(msg.instance().is_none());
    assert!(msg.oil_pressure().is_none());
    assert!(msg.oil_temperature().is_none());
    assert!(msg.temperature().is_none());
    assert!(msg.total_engine_hours().is_none());
    assert!(msg.coolant_pressure().is_none());
    assert!(msg.fuel_pressure().is_none());
    assert!(msg.discrete_status1().is_none());
    assert!(msg.discrete_status2().is_none());
    // Signed: all-ones = -1 (valid value)
    assert!(msg.alternator_potential().is_some());
    assert!(msg.fuel_rate().is_some());
    assert!(msg.engine_load().is_some());
    assert!(msg.engine_torque().is_some());
}

/// All-zeros payload gives valid field values.
#[test]
fn all_zeros_are_valid() {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let data = [0x00; 8];
    let msg = VesselHeading::decode(&data).unwrap();
    assert_eq!(msg.sid(), Some(0));
    assert_eq!(msg.heading(), Some(0.0));
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 5: Encode/decode roundtrip
// canboat only tests decoding; these verify our encoding path.
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn roundtrip_vessel_heading() {
    use nmea2000_pgns::vessel_heading::VesselHeading;

    let msg = VesselHeading::builder().sid(42).heading(1000.0).build();

    let mut buf = [0u8; 8];
    let len = msg.encode(&mut buf).unwrap();
    assert_eq!(len, 8);
    let decoded = VesselHeading::decode(&buf).unwrap();
    assert_eq!(decoded.sid(), Some(42));
}

#[test]
fn roundtrip_wind_data() {
    use nmea2000_pgns::wind_data::WindData;

    let msg = WindData::builder()
        .sid(7)
        .wind_speed(15.5)
        .wind_angle(1.0)
        .build();

    let mut buf = [0u8; 8];
    let len = msg.encode(&mut buf).unwrap();
    assert_eq!(len, 8);
    let decoded = WindData::decode(&buf).unwrap();
    assert_eq!(decoded.sid(), Some(7));
    assert_near(
        decoded.wind_speed().unwrap(),
        15.50,
        0.01,
        "wind_speed roundtrip",
    );
    assert_near(
        decoded.wind_angle().unwrap(),
        1.0,
        0.001,
        "wind_angle roundtrip",
    );
}

#[test]
fn roundtrip_battery_status() {
    use nmea2000_pgns::battery_status::BatteryStatus;

    let msg = BatteryStatus::builder()
        .instance(1)
        .voltage(13.80)
        .current(-2.5)
        .build();

    let mut buf = [0u8; 8];
    let len = msg.encode(&mut buf).unwrap();
    assert_eq!(len, 8);
    let decoded = BatteryStatus::decode(&buf).unwrap();
    assert_eq!(decoded.instance(), Some(1));
    assert_near(decoded.voltage().unwrap(), 13.80, 0.01, "voltage roundtrip");
    assert_near(decoded.current().unwrap(), -2.5, 0.1, "current roundtrip");
}

#[test]
fn roundtrip_water_depth() {
    use nmea2000_pgns::water_depth::WaterDepth;

    let msg = WaterDepth::builder()
        .sid(0)
        .depth(25.50)
        .offset(-1.0)
        .range(500.0)
        .build();

    let mut buf = [0u8; 8];
    let len = msg.encode(&mut buf).unwrap();
    assert_eq!(len, 8);
    let decoded = WaterDepth::decode(&buf).unwrap();
    assert_eq!(decoded.sid(), Some(0));
    assert_near(decoded.depth().unwrap(), 25.50, 0.01, "depth roundtrip");
    assert_near(decoded.offset().unwrap(), -1.0, 0.001, "offset roundtrip");
    assert_near(decoded.range().unwrap(), 500.0, 10.0, "range roundtrip");
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 6: Error handling
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn insufficient_data_returns_error() {
    use nmea2000_pgns::vessel_heading::VesselHeading;
    let data = [0x00, 0x9F, 0x01, 0xFF];
    assert!(VesselHeading::decode(&data).is_err());
}

// ═══════════════════════════════════════════════════════════════════════
// SECTION 7: Multi-variant PGN dispatch
// ═══════════════════════════════════════════════════════════════════════

#[test]
fn dispatch_pgn_126208_request() {
    // functionCode = 0 → NmeaRequestGroupFunction
    let data = [0x00, 0x00, 0xEE, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x01];
    let msg = DecodedMessage::decode(126208, &data).unwrap();
    assert!(
        matches!(msg, DecodedMessage::NmeaRequestGroupFunction(_)),
        "Expected NmeaRequestGroupFunction, got {:?}",
        msg.pgn()
    );
}

#[test]
fn dispatch_pgn_126208_command() {
    // functionCode = 1 → NmeaCommandGroupFunction
    let data = [0x01, 0x00, 0xEE, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x01];
    let msg = DecodedMessage::decode(126208, &data).unwrap();
    assert!(
        matches!(msg, DecodedMessage::NmeaCommandGroupFunction(_)),
        "Expected NmeaCommandGroupFunction, got {:?}",
        msg.pgn()
    );
}

#[test]
fn dispatch_pgn_126208_acknowledge() {
    // functionCode = 2 → NmeaAcknowledgeGroupFunction
    let data = [0x02, 0x00, 0xEE, 0x00, 0x01, 0x00, 0x01, 0x00];
    let msg = DecodedMessage::decode(126208, &data).unwrap();
    assert!(
        matches!(msg, DecodedMessage::NmeaAcknowledgeGroupFunction(_)),
        "Expected NmeaAcknowledgeGroupFunction, got {:?}",
        msg.pgn()
    );
}

#[test]
fn dispatch_pgn_126208_read_fields() {
    // functionCode = 3 → NmeaReadFieldsGroupFunction
    let data = [
        0x03, 0x00, 0xEE, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x01, 0x01,
    ];
    let msg = DecodedMessage::decode(126208, &data).unwrap();
    assert!(
        matches!(msg, DecodedMessage::NmeaReadFieldsGroupFunction(_)),
        "Expected NmeaReadFieldsGroupFunction, got {:?}",
        msg.pgn()
    );
}

#[test]
fn dispatch_pgn_126208_unknown_function_code() {
    // functionCode = 99 → Unknown (no matching variant)
    let data = [99, 0x00, 0xEE, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
    let msg = DecodedMessage::decode(126208, &data).unwrap();
    assert!(
        matches!(msg, DecodedMessage::Unknown { .. }),
        "Expected Unknown for unrecognized function code"
    );
}

#[test]
fn dispatch_pgn_126208_all_variants_correct_pgn() {
    // All 126208 variants should report PGN 126208
    for fc in 0..=6u8 {
        let data = [
            fc, 0x00, 0xEE, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x01, 0x01, 0x01,
        ];
        let msg = DecodedMessage::decode(126208, &data).unwrap();
        assert_eq!(
            msg.pgn(),
            nmea2000_types::Pgn::new(126208),
            "function_code {fc} should report PGN 126208"
        );
    }
}

// ──────────────────────────────────────────────────
// Enhanced Debug output
// ──────────────────────────────────────────────────

#[test]
fn debug_shows_decoded_fields() {
    use nmea2000_pgns::vessel_heading::VesselHeading;
    use nmea2000_types::PgnMessage;

    // SID=0, Heading=0.0415 rad, deviation=unknown, variation=unknown, reference=Magnetic
    let data = [0x00, 0x9F, 0x01, 0xFF, 0x7F, 0xFF, 0x7F, 0x00];
    let msg = VesselHeading::decode(&data).unwrap();
    let dbg = format!("{msg:?}");

    // Should contain field names with decoded values, not raw bytes
    assert!(
        dbg.contains("sid:"),
        "Debug should contain 'sid:' field, got: {dbg}"
    );
    assert!(
        dbg.contains("heading:"),
        "Debug should contain 'heading:' field, got: {dbg}"
    );
    assert!(
        dbg.contains("reference:"),
        "Debug should contain 'reference:' field, got: {dbg}"
    );
    // Should NOT contain raw byte arrays
    assert!(
        !dbg.contains("[0,"),
        "Debug should not show raw byte array, got: {dbg}"
    );
}

// ──────────────────────────────────────────────────
// STRING_FIX fields
// ──────────────────────────────────────────────────

/// Decode AIS Class B static data (msg 24 Part A) with a known name.
/// Name field is 20 bytes at offset 5, padded with '@' (AIS convention).
#[test]
fn string_fix_decode_ais_name() {
    use nmea2000_pgns::ais_class_b_static_data_msg24_part_a::AisClassBStaticDataMsg24PartA;
    use nmea2000_types::PgnMessage;

    let mut data = [0u8; 27];
    // Message ID = 24 (6 bits), Repeat = 0 (2 bits)
    data[0] = 24;
    // User ID (MMSI) = 123456789 at bits 8..40
    let mmsi: u32 = 123456789;
    data[1..5].copy_from_slice(&mmsi.to_le_bytes());
    // Name at bytes 5..25: "ENTERPRISE" padded with '@'
    let name_bytes = b"ENTERPRISE@@@@@@@@@@";
    data[5..25].copy_from_slice(name_bytes);

    let msg = AisClassBStaticDataMsg24PartA::decode(&data).unwrap();
    assert_eq!(msg.name(), Some("ENTERPRISE"));
    assert_eq!(msg.name_raw(), b"ENTERPRISE@@@@@@@@@@");
    assert_eq!(msg.user_id(), Some(123456789));
}

/// Roundtrip: build with builder, decode, verify string field.
#[test]
fn string_fix_builder_roundtrip() {
    use nmea2000_pgns::ais_class_b_static_data_msg24_part_a::AisClassBStaticDataMsg24PartA;
    use nmea2000_types::PgnMessage;

    let msg = AisClassBStaticDataMsg24PartA::builder()
        .name("TESTSHIP")
        .sequence_id(42)
        .build();

    let mut buf = [0u8; 27];
    msg.encode(&mut buf).unwrap();
    let decoded = AisClassBStaticDataMsg24PartA::decode(&buf).unwrap();

    assert_eq!(decoded.name(), Some("TESTSHIP"));
    assert_eq!(decoded.sequence_id(), Some(42));
}

/// All-0xFF data → name() returns None (all padding).
#[test]
fn string_fix_all_unknown() {
    use nmea2000_pgns::ais_class_b_static_data_msg24_part_a::AisClassBStaticDataMsg24PartA;
    use nmea2000_types::PgnMessage;

    let data = [0xFF; 27];
    let msg = AisClassBStaticDataMsg24PartA::decode(&data).unwrap();
    assert_eq!(msg.name(), None);
}

/// Debug output shows string value, not raw bytes.
#[test]
fn string_fix_debug_shows_string() {
    use nmea2000_pgns::ais_class_b_static_data_msg24_part_a::AisClassBStaticDataMsg24PartA;
    use nmea2000_types::PgnMessage;

    let msg = AisClassBStaticDataMsg24PartA::builder()
        .name("MYSHIP")
        .build();

    let dbg = format!("{msg:?}");
    assert!(
        dbg.contains("MYSHIP"),
        "Debug should show string value, got: {dbg}"
    );
}
