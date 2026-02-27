use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;

use nmea2000::bus::{CanTransport, N2kBus};
use nmea2000_types::{CanId, FastPacketFragmenter, IsoTpBamFragmenter, Pgn};

/// Mock CAN transport backed by queues.
struct MockTransport {
    rx: RefCell<VecDeque<(CanId, Vec<u8>)>>,
    tx: RefCell<Vec<(CanId, Vec<u8>)>>,
}

impl MockTransport {
    fn new(frames: Vec<(CanId, Vec<u8>)>) -> Self {
        Self {
            rx: RefCell::new(frames.into()),
            tx: RefCell::new(Vec::new()),
        }
    }

    fn sent_frames(&self) -> Vec<(CanId, Vec<u8>)> {
        self.tx.borrow().clone()
    }
}

impl CanTransport for MockTransport {
    fn send(&self, id: CanId, data: &[u8]) -> io::Result<()> {
        self.tx.borrow_mut().push((id, data.to_vec()));
        Ok(())
    }

    fn receive(&self) -> io::Result<(CanId, Vec<u8>)> {
        self.rx
            .borrow_mut()
            .pop_front()
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no more frames"))
    }

    fn try_receive(&self) -> io::Result<Option<(CanId, Vec<u8>)>> {
        Ok(self.rx.borrow_mut().pop_front())
    }
}

// PGN 59904 (ISO Request) is PacketType::Single — good for single-frame tests.
const SINGLE_FRAME_PGN: u32 = 59904;

#[test]
fn single_frame_read() {
    let pgn = Pgn::new(SINGLE_FRAME_PGN);
    let id = CanId::new(6, pgn, 255, 10);
    let data = vec![0x11, 0x22, 0x33];

    let transport = MockTransport::new(vec![(id, data.clone())]);
    let mut bus = N2kBus::new(transport);

    let msg = bus.read_message().unwrap();
    assert_eq!(msg.pgn, pgn);
    assert_eq!(msg.source, 10);
    assert_eq!(msg.priority, 6);
    assert_eq!(msg.data, data);
}

#[test]
fn single_frame_try_read_empty() {
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let result = bus.try_read_message().unwrap();
    assert!(result.is_none());
}

#[test]
fn fast_packet_reassembly() {
    let pgn = Pgn::new(129029); // GNSS Position Data — fast packet
    let source = 5;
    let original: Vec<u8> = (0..20).collect();

    let fragmenter = FastPacketFragmenter::new(pgn, source, 3, 255, &original, 0);
    let frames: Vec<(CanId, Vec<u8>)> = fragmenter
        .map(|(id, data, len)| (id, data[..len].to_vec()))
        .collect();

    let transport = MockTransport::new(frames);
    let mut bus = N2kBus::new(transport);

    let msg = bus.read_message().unwrap();
    assert_eq!(msg.pgn, pgn);
    assert_eq!(msg.source, source);
    assert_eq!(msg.data, original);
}

#[test]
fn send_single_frame() {
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let pgn = Pgn::new(SINGLE_FRAME_PGN);
    let data = vec![0xAA, 0xBB, 0xCC];
    bus.send_raw(pgn, 10, 6, 255, &data).unwrap();

    let sent = bus.transport().sent_frames();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, data);
}

#[test]
fn send_fast_packet_roundtrip() {
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let pgn = Pgn::new(129029);
    let data: Vec<u8> = (0..20).collect();
    bus.send_raw(pgn, 5, 3, 255, &data).unwrap();

    let sent = bus.transport().sent_frames();
    // 20 bytes = first frame (6 data) + 2 continuation frames (7 each)
    assert_eq!(sent.len(), 3);

    // Reassemble to verify roundtrip
    let mut bus2 = N2kBus::new(MockTransport::new(sent));
    let msg = bus2.read_message().unwrap();
    assert_eq!(msg.pgn, pgn);
    assert_eq!(msg.data, data);
}

#[test]
fn decode_known_pgn() {
    let pgn = Pgn::new(SINGLE_FRAME_PGN); // ISO Request (3 bytes: requested PGN LE)
    let id = CanId::new(6, pgn, 255, 10);
    // Requesting PGN 60928 (0x00EE00) in little-endian
    let data = vec![0x00, 0xEE, 0x00];

    let transport = MockTransport::new(vec![(id, data)]);
    let mut bus = N2kBus::new(transport);

    let raw = bus.read_message().unwrap();
    let decoded = N2kBus::<MockTransport>::decode(&raw).unwrap();
    assert_eq!(decoded.pgn(), Pgn::new(SINGLE_FRAME_PGN));
}

#[test]
fn send_iso_tp_bam() {
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let pgn = Pgn::new(126996); // Product Information — ISO TP
    let data: Vec<u8> = (0..=255).cycle().take(300).collect();
    bus.send_raw(pgn, 10, 6, 255, &data).unwrap();

    let sent = bus.transport().sent_frames();
    // First frame is BAM announcement, then ceil(300/7) = 43 data frames
    assert_eq!(sent.len(), 44);

    // BAM announcement frame
    let (bam_id, bam_data) = &sent[0];
    assert_eq!(bam_id.pgn(), Pgn::ISO_TP_CONNECTION_MGMT);
    assert_eq!(bam_data[0], 0x20); // BAM control byte
    let announced_len = u16::from_le_bytes([bam_data[1], bam_data[2]]) as usize;
    assert_eq!(announced_len, 300);

    // Data frames use ISO TP Data Transfer PGN
    for (id, _) in &sent[1..] {
        assert_eq!(id.pgn(), Pgn::ISO_TP_DATA_TRANSFER);
    }
}

#[test]
fn send_iso_tp_bam_roundtrip() {
    // Send 300 bytes via BAM, then reassemble on receive side
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let pgn = Pgn::new(126996);
    let data: Vec<u8> = (0..=255).cycle().take(300).collect();
    bus.send_raw(pgn, 10, 6, 255, &data).unwrap();

    let sent = bus.transport().sent_frames();

    // Feed sent frames into a new bus for reassembly
    let mut bus2 = N2kBus::new(MockTransport::new(sent));
    let msg = bus2.read_message().unwrap();
    assert_eq!(msg.pgn, pgn);
    assert_eq!(msg.source, 10);
    assert_eq!(msg.data, data);
}

#[test]
fn send_boundary_fast_packet_vs_bam() {
    // 223 bytes should still use Fast Packet
    let transport = MockTransport::new(vec![]);
    let mut bus = N2kBus::new(transport);

    let pgn = Pgn::new(129029);
    let data: Vec<u8> = vec![0xAA; 223];
    bus.send_raw(pgn, 5, 3, 255, &data).unwrap();

    let sent = bus.transport().sent_frames();
    // Fast Packet: first frame carries 6 bytes, rest carry 7 each
    // (223 - 6) / 7 = 31 continuation frames + 1 first = 32 total
    assert_eq!(sent.len(), 32);
    // Verify it's Fast Packet (not BAM) — first byte of first frame has sequence counter
    assert_ne!(sent[0].1[0], 0x20); // Not BAM announcement

    // 224 bytes should use ISO TP BAM
    let transport2 = MockTransport::new(vec![]);
    let mut bus2 = N2kBus::new(transport2);
    let data2: Vec<u8> = vec![0xBB; 224];
    bus2.send_raw(pgn, 5, 3, 255, &data2).unwrap();

    let sent2 = bus2.transport().sent_frames();
    // BAM: first frame is announcement
    assert_eq!(sent2[0].1[0], 0x20); // BAM control byte
}

#[test]
fn rts_receive_sends_cts() {
    // Simulate receiving an RTS and verify bus sends CTS in response
    let source = 10;
    let our_addr = 0xFF; // broadcast destination for now
    let target_pgn = Pgn::new(65240);
    let pgn_raw = target_pgn.as_u32();

    // Create an RTS frame
    let cm_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, our_addr, source);
    let rts = vec![
        16u8, // RTS
        14,
        0, // 14 bytes
        2, // 2 packets
        0xFF,
        (pgn_raw & 0xFF) as u8,
        ((pgn_raw >> 8) & 0xFF) as u8,
        ((pgn_raw >> 16) & 0xFF) as u8,
    ];

    let transport = MockTransport::new(vec![(cm_id, rts)]);
    let mut bus = N2kBus::new(transport);

    // try_read_message processes the RTS and should send a CTS response
    let result = bus.try_read_message().unwrap();
    assert!(result.is_none()); // No complete message yet

    let sent = bus.transport().sent_frames();
    assert_eq!(sent.len(), 1, "Should have sent 1 CTS frame");

    let (cts_id, cts_data) = &sent[0];
    assert_eq!(cts_id.pgn(), Pgn::ISO_TP_CONNECTION_MGMT);
    assert_eq!(cts_data[0], 17); // CTS control byte
    assert_eq!(cts_data[1], 2); // max_packets
    assert_eq!(cts_data[2], 1); // next_seq
}

#[test]
fn rts_cts_full_session() {
    // Full RTS/CTS session: RTS → CTS → Data → EOM_ACK
    let source = 10;
    let our_addr = 0xFF;
    let target_pgn = Pgn::new(65240);
    let pgn_raw = target_pgn.as_u32();

    // Build the frames the remote peer sends
    let cm_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, our_addr, source);
    let rts = vec![
        16u8,
        14,
        0,
        2,
        0xFF,
        (pgn_raw & 0xFF) as u8,
        ((pgn_raw >> 8) & 0xFF) as u8,
        ((pgn_raw >> 16) & 0xFF) as u8,
    ];

    let dt_id = CanId::new(6, Pgn::ISO_TP_DATA_TRANSFER, our_addr, source);
    let dt1 = vec![1u8, 0, 1, 2, 3, 4, 5, 6];
    let dt2 = vec![2u8, 7, 8, 9, 10, 11, 12, 13];

    let frames = vec![(cm_id, rts), (dt_id, dt1), (dt_id, dt2)];

    let transport = MockTransport::new(frames);
    let mut bus = N2kBus::new(transport);

    // Read the complete message (processes RTS, sends CTS, then data, sends EOM_ACK)
    let msg = bus.read_message().unwrap();
    assert_eq!(msg.pgn, target_pgn);
    assert_eq!(msg.source, source);
    assert_eq!(msg.data, (0..14).collect::<Vec<u8>>());

    let sent = bus.transport().sent_frames();
    // Should have sent CTS + EOM_ACK = 2 frames
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].1[0], 17); // CTS
    assert_eq!(sent[1].1[0], 19); // EOM_ACK
    // EOM_ACK should contain total bytes
    assert_eq!(sent[1].1[1], 14); // total_bytes low
    assert_eq!(sent[1].1[2], 0); // total_bytes high
    assert_eq!(sent[1].1[3], 2); // total_packets
}

#[test]
fn bam_receive_no_cts() {
    // BAM session should NOT send CTS or EOM_ACK
    let source = 42;
    let target_pgn = Pgn::new(65240);
    let original: Vec<u8> = (0..14).collect();

    let frames: Vec<(CanId, Vec<u8>)> = IsoTpBamFragmenter::new(source, target_pgn, &original)
        .map(|(id, data, len)| (id, data[..len].to_vec()))
        .collect();

    let transport = MockTransport::new(frames);
    let mut bus = N2kBus::new(transport);

    let msg = bus.read_message().unwrap();
    assert_eq!(msg.pgn, target_pgn);
    assert_eq!(msg.data, original);

    // BAM: no frames should have been sent
    let sent = bus.transport().sent_frames();
    assert!(
        sent.is_empty(),
        "BAM should not send CTS/ACK, but sent {} frames",
        sent.len()
    );
}
