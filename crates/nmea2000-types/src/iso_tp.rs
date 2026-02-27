use crate::error::IsoTpError;
use crate::id::CanId;
use crate::pgn::Pgn;

/// Maximum ISO TP payload size (1785 bytes = 255 * 7).
pub const MAX_ISO_TP_SIZE: usize = 1785;

/// Maximum concurrent receive sessions.
const MAX_RX_SESSIONS: usize = 8;

/// ISO TP Connection Management control bytes.
#[allow(dead_code)]
mod cm {
    pub const RTS: u8 = 16; // Request To Send
    pub const CTS: u8 = 17; // Clear To Send
    pub const EOM_ACK: u8 = 19; // End of Message Acknowledge
    pub const BAM: u8 = 32; // Broadcast Announce Message
    pub const ABORT: u8 = 255;
}

/// A completed ISO TP message.
#[derive(Debug)]
pub struct IsoTpComplete {
    pub pgn: Pgn,
    pub source: u8,
    pub destination: u8,
    pub data: Vec<u8>,
}

/// Action to take after processing an ISO TP frame.
#[derive(Debug)]
pub enum IsoTpAction {
    /// No response needed; reassembly is in progress.
    None,
    /// A complete message was reassembled (BAM mode, no acknowledgement needed).
    Complete(IsoTpComplete),
    /// Send a CTS (Clear To Send) response to the peer.
    SendCts {
        destination: u8,
        max_packets: u8,
        next_seq: u8,
        pgn: Pgn,
    },
    /// A complete message was reassembled (RTS/CTS mode); send an `EOM_ACK`.
    CompleteWithAck {
        complete: IsoTpComplete,
        destination: u8,
        total_bytes: u16,
        total_packets: u8,
        pgn: Pgn,
    },
}

/// Receive session state.
struct RxSession {
    source: u8,
    destination: u8,
    pgn: Pgn,
    total_bytes: usize,
    total_packets: u8,
    /// Whether this is a BAM session (no CTS/ACK needed).
    is_bam: bool,
    /// Bitmask tracking which packets have been received (1-indexed).
    /// 256 bits covers `seq_number` 1..=255.
    received_mask: [u64; 4],
    buffer: Vec<u8>,
}

impl RxSession {
    /// Check whether a packet has already been received.
    fn has_packet(&self, seq: u8) -> bool {
        let bit = seq as usize;
        self.received_mask[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    /// Mark a packet as received.
    fn mark_received(&mut self, seq: u8) {
        let bit = seq as usize;
        self.received_mask[bit / 64] |= 1u64 << (bit % 64);
    }

    /// Check whether all packets `1..=total_packets` have been received.
    fn is_complete(&self) -> bool {
        for seq in 1..=self.total_packets {
            if !self.has_packet(seq) {
                return false;
            }
        }
        true
    }
}

/// Sentinel value for empty session slots.
const NONE_SESSION: Option<RxSession> = None;

/// ISO 11783-3 Transport Protocol manager.
///
/// Handles reassembly of multi-packet messages using both the
/// connection-oriented (RTS/CTS) and broadcast (BAM) modes.
/// Supports out-of-order packet reception.
pub struct IsoTpManager {
    rx_sessions: [Option<RxSession>; MAX_RX_SESSIONS],
}

impl IsoTpManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rx_sessions: [NONE_SESSION; MAX_RX_SESSIONS],
        }
    }

    /// Process an incoming CAN frame.
    ///
    /// Handles both Connection Management (PGN 60416) and Data Transfer (PGN 60160) frames.
    /// Returns an action indicating what response (if any) should be sent,
    /// and whether a complete message was reassembled.
    ///
    /// # Errors
    ///
    /// Returns `IsoTpError` if the frame is too short, the session state is invalid,
    /// or the connection was aborted by the peer.
    pub fn process_frame(&mut self, id: CanId, data: &[u8]) -> Result<IsoTpAction, IsoTpError> {
        let pgn = id.pgn();

        if pgn == Pgn::ISO_TP_CONNECTION_MGMT {
            self.process_connection_management(id, data)
        } else if pgn == Pgn::ISO_TP_DATA_TRANSFER {
            self.process_data_transfer(id, data)
        } else {
            Ok(IsoTpAction::None)
        }
    }

    fn process_connection_management(
        &mut self,
        id: CanId,
        data: &[u8],
    ) -> Result<IsoTpAction, IsoTpError> {
        if data.len() < 8 {
            return Err(IsoTpError::InvalidState);
        }

        let control = data[0];
        let source = id.source_address();
        let destination = id.destination().unwrap_or(0xFF);

        match control {
            cm::RTS | cm::BAM => {
                let is_bam = control == cm::BAM;
                let total_bytes = u16::from_le_bytes([data[1], data[2]]) as usize;
                let total_packets = data[3];
                let pgn_bytes = [data[5], data[6], data[7]];
                let target_pgn = Pgn::new(
                    u32::from(pgn_bytes[0])
                        | (u32::from(pgn_bytes[1]) << 8)
                        | (u32::from(pgn_bytes[2]) << 16),
                );

                if total_bytes > MAX_ISO_TP_SIZE {
                    return Err(IsoTpError::InvalidState);
                }

                let slot = self.find_or_allocate(source, target_pgn)?;
                *slot = Some(RxSession {
                    source,
                    destination,
                    pgn: target_pgn,
                    total_bytes,
                    total_packets,
                    is_bam,
                    received_mask: [0; 4],
                    buffer: vec![0u8; total_bytes],
                });

                if is_bam {
                    Ok(IsoTpAction::None)
                } else {
                    // RTS received — respond with CTS to request all packets
                    Ok(IsoTpAction::SendCts {
                        destination: source,
                        max_packets: total_packets,
                        next_seq: 1,
                        pgn: target_pgn,
                    })
                }
            }
            cm::ABORT => {
                self.remove_session(source, destination);
                Err(IsoTpError::Abort(data[1]))
            }
            _ => Ok(IsoTpAction::None),
        }
    }

    fn process_data_transfer(&mut self, id: CanId, data: &[u8]) -> Result<IsoTpAction, IsoTpError> {
        if data.len() < 2 {
            return Err(IsoTpError::InvalidState);
        }

        let source = id.source_address();
        let seq_number = data[0];
        let payload = &data[1..];

        // Find matching session by source
        let session = self
            .rx_sessions
            .iter_mut()
            .flatten()
            .find(|s| s.source == source);

        let Some(session) = session else {
            return Err(IsoTpError::InvalidState);
        };

        // Validate sequence number range
        if seq_number == 0 || seq_number > session.total_packets {
            return Err(IsoTpError::InvalidState);
        }

        // Ignore duplicate packets
        if session.has_packet(seq_number) {
            return Ok(IsoTpAction::None);
        }

        // Copy payload data into the correct buffer position
        let offset = (seq_number as usize - 1) * 7;
        let remaining = session.total_bytes.saturating_sub(offset);
        let copy_len = payload.len().min(remaining);
        if offset + copy_len <= session.buffer.len() {
            session.buffer[offset..offset + copy_len].copy_from_slice(&payload[..copy_len]);
        }
        session.mark_received(seq_number);

        // Check if complete
        if session.is_complete() {
            let is_bam = session.is_bam;
            let total_bytes = session.total_bytes as u16;
            let total_packets = session.total_packets;
            let session_pgn = session.pgn;
            let session_destination = session.destination;
            let complete = IsoTpComplete {
                pgn: session.pgn,
                source: session.source,
                destination: session.destination,
                data: session.buffer[..session.total_bytes].to_vec(),
            };
            let src = session.source;
            self.remove_session_by_pgn(src, session_pgn);

            if is_bam {
                Ok(IsoTpAction::Complete(complete))
            } else {
                Ok(IsoTpAction::CompleteWithAck {
                    complete,
                    destination: session_destination,
                    total_bytes,
                    total_packets,
                    pgn: session_pgn,
                })
            }
        } else {
            Ok(IsoTpAction::None)
        }
    }

    fn find_or_allocate(
        &mut self,
        source: u8,
        pgn: Pgn,
    ) -> Result<&mut Option<RxSession>, IsoTpError> {
        let mut existing = None;
        let mut empty = None;
        for (i, slot) in self.rx_sessions.iter().enumerate() {
            if let Some(session) = slot {
                if session.source == source && session.pgn == pgn {
                    existing = Some(i);
                    break;
                }
            } else if empty.is_none() {
                empty = Some(i);
            }
        }

        if let Some(idx) = existing {
            Ok(&mut self.rx_sessions[idx])
        } else if let Some(idx) = empty {
            Ok(&mut self.rx_sessions[idx])
        } else {
            Err(IsoTpError::BufferFull)
        }
    }

    fn remove_session(&mut self, source: u8, destination: u8) {
        for slot in &mut self.rx_sessions {
            if let Some(session) = slot
                && session.source == source
                && session.destination == destination
            {
                *slot = None;
                return;
            }
        }
    }

    fn remove_session_by_pgn(&mut self, source: u8, pgn: Pgn) {
        for slot in &mut self.rx_sessions {
            if let Some(session) = slot
                && session.source == source
                && session.pgn == pgn
            {
                *slot = None;
                return;
            }
        }
    }
}

impl Default for IsoTpManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Fragment a message for ISO TP BAM transmission.
///
/// Returns an iterator of CAN frames: first a BAM announcement, then data transfer frames.
pub struct IsoTpBamFragmenter<'a> {
    source: u8,
    target_pgn: Pgn,
    data: &'a [u8],
    state: BamState,
    packet_index: u8,
    data_offset: usize,
}

enum BamState {
    Announcement,
    Data,
    Done,
}

impl<'a> IsoTpBamFragmenter<'a> {
    #[must_use]
    pub fn new(source: u8, target_pgn: Pgn, data: &'a [u8]) -> Self {
        Self {
            source,
            target_pgn,
            data,
            state: BamState::Announcement,
            packet_index: 1,
            data_offset: 0,
        }
    }
}

impl Iterator for IsoTpBamFragmenter<'_> {
    type Item = (CanId, [u8; 8], usize);

    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            BamState::Announcement => {
                let total_packets = self.data.len().div_ceil(7) as u8;
                let pgn_raw = self.target_pgn.as_u32();

                let can_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, 0xFF, self.source);
                let frame = [
                    cm::BAM,
                    (self.data.len() & 0xFF) as u8,
                    ((self.data.len() >> 8) & 0xFF) as u8,
                    total_packets,
                    0xFF, // Reserved
                    (pgn_raw & 0xFF) as u8,
                    ((pgn_raw >> 8) & 0xFF) as u8,
                    ((pgn_raw >> 16) & 0xFF) as u8,
                ];
                self.state = BamState::Data;
                Some((can_id, frame, 8))
            }
            BamState::Data => {
                if self.data_offset >= self.data.len() {
                    self.state = BamState::Done;
                    return None;
                }

                let can_id = CanId::new(6, Pgn::ISO_TP_DATA_TRANSFER, 0xFF, self.source);
                let mut frame = [0xFFu8; 8];
                frame[0] = self.packet_index;

                let remaining = self.data.len() - self.data_offset;
                let copy_len = remaining.min(7);
                frame[1..=copy_len]
                    .copy_from_slice(&self.data[self.data_offset..self.data_offset + copy_len]);

                self.data_offset += copy_len;

                if self.data_offset >= self.data.len() {
                    self.state = BamState::Done;
                } else {
                    self.packet_index += 1;
                }

                Some((can_id, frame, 8))
            }
            BamState::Done => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bam_roundtrip() {
        let target_pgn = Pgn::new(65240);
        let source = 42;
        let original: Vec<u8> = (0..25).collect();

        // Fragment
        let frames: Vec<_> = IsoTpBamFragmenter::new(source, target_pgn, &original).collect();

        // Should be 1 BAM announcement + ceil(25/7) = 4 data frames = 5 total
        assert_eq!(frames.len(), 5);

        // First frame should be BAM CM
        assert_eq!(frames[0].1[0], cm::BAM);
        assert_eq!(frames[0].1[1], 25); // total bytes low
        assert_eq!(frames[0].1[3], 4); // total packets

        // Reassemble
        let mut manager = IsoTpManager::new();
        let mut result = None;
        for (id, data, len) in &frames {
            match manager.process_frame(*id, &data[..*len]) {
                Ok(IsoTpAction::Complete(complete)) => result = Some(complete),
                Ok(IsoTpAction::None) => {}
                Err(e) => panic!("Error: {e}"),
                other => panic!("Unexpected action: {other:?}"),
            }
        }

        let complete = result.expect("Should have reassembled");
        assert_eq!(complete.pgn, target_pgn);
        assert_eq!(complete.source, source);
        assert_eq!(complete.data, original);
    }

    #[test]
    fn rts_session_setup() {
        let mut manager = IsoTpManager::new();
        let source = 10;
        let target_pgn = Pgn::new(65240);

        // Send RTS — should get SendCts response
        let cm_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, 0xFF, source);
        let pgn_raw = target_pgn.as_u32();
        let rts = [
            cm::RTS,
            14,
            0, // 14 bytes total
            2, // 2 packets
            0xFF,
            (pgn_raw & 0xFF) as u8,
            ((pgn_raw >> 8) & 0xFF) as u8,
            ((pgn_raw >> 16) & 0xFF) as u8,
        ];
        let action = manager.process_frame(cm_id, &rts).unwrap();
        match action {
            IsoTpAction::SendCts {
                destination,
                max_packets,
                next_seq,
                pgn,
            } => {
                assert_eq!(destination, source);
                assert_eq!(max_packets, 2);
                assert_eq!(next_seq, 1);
                assert_eq!(pgn, target_pgn);
            }
            other => panic!("Expected SendCts, got {other:?}"),
        }

        // Send data packets
        let dt_id = CanId::new(6, Pgn::ISO_TP_DATA_TRANSFER, 0xFF, source);
        let dt1 = [1, 0, 1, 2, 3, 4, 5, 6];
        assert!(matches!(
            manager.process_frame(dt_id, &dt1).unwrap(),
            IsoTpAction::None
        ));

        // Final data packet — should get CompleteWithAck
        let dt2 = [2, 7, 8, 9, 10, 11, 12, 13];
        let action = manager.process_frame(dt_id, &dt2).unwrap();
        match action {
            IsoTpAction::CompleteWithAck {
                complete,
                total_bytes,
                total_packets,
                pgn,
                ..
            } => {
                assert_eq!(complete.pgn, target_pgn);
                assert_eq!(complete.data, (0..14).collect::<Vec<u8>>());
                assert_eq!(total_bytes, 14);
                assert_eq!(total_packets, 2);
                assert_eq!(pgn, target_pgn);
            }
            other => panic!("Expected CompleteWithAck, got {other:?}"),
        }
    }

    #[test]
    fn bam_out_of_order() {
        let target_pgn = Pgn::new(65240);
        let source = 42;
        let original: Vec<u8> = (0..25).collect();

        // Fragment in order
        let frames: Vec<_> = IsoTpBamFragmenter::new(source, target_pgn, &original).collect();
        assert_eq!(frames.len(), 5); // 1 BAM + 4 data

        // Reassemble with data frames in reverse order
        let mut manager = IsoTpManager::new();

        // Send BAM announcement first
        let (id, data, len) = &frames[0];
        assert!(matches!(
            manager.process_frame(*id, &data[..*len]).unwrap(),
            IsoTpAction::None
        ));

        // Send data frames in reverse: 4, 3, 2, 1
        for i in [4, 3, 2, 1] {
            let (id, data, len) = &frames[i];
            let action = manager.process_frame(*id, &data[..*len]).unwrap();
            if i == 1 {
                // Last packet to arrive completes the message
                match action {
                    IsoTpAction::Complete(complete) => {
                        assert_eq!(complete.pgn, target_pgn);
                        assert_eq!(complete.source, source);
                        assert_eq!(complete.data, original);
                    }
                    other => panic!("Expected Complete, got {other:?}"),
                }
            } else {
                assert!(matches!(action, IsoTpAction::None));
            }
        }
    }

    #[test]
    fn bam_duplicate_ignored() {
        let target_pgn = Pgn::new(65240);
        let source = 42;
        let original: Vec<u8> = (0..14).collect();

        let frames: Vec<_> = IsoTpBamFragmenter::new(source, target_pgn, &original).collect();
        assert_eq!(frames.len(), 3); // 1 BAM + 2 data

        let mut manager = IsoTpManager::new();

        // Send BAM
        let (id, data, len) = &frames[0];
        manager.process_frame(*id, &data[..*len]).unwrap();

        // Send packet 1
        let (id, data, len) = &frames[1];
        assert!(matches!(
            manager.process_frame(*id, &data[..*len]).unwrap(),
            IsoTpAction::None
        ));

        // Send packet 1 again (duplicate) — should be silently ignored
        let (id, data, len) = &frames[1];
        assert!(matches!(
            manager.process_frame(*id, &data[..*len]).unwrap(),
            IsoTpAction::None
        ));

        // Send packet 2 — should complete
        let (id, data, len) = &frames[2];
        match manager.process_frame(*id, &data[..*len]).unwrap() {
            IsoTpAction::Complete(complete) => assert_eq!(complete.data, original),
            other => panic!("Expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn bam_retransmission() {
        let target_pgn = Pgn::new(65240);
        let source = 42;
        let original: Vec<u8> = (0..14).collect();

        let frames: Vec<_> = IsoTpBamFragmenter::new(source, target_pgn, &original).collect();

        let mut manager = IsoTpManager::new();

        // First attempt: BAM + packet 1 only (incomplete)
        let (id, data, len) = &frames[0];
        manager.process_frame(*id, &data[..*len]).unwrap();
        let (id, data, len) = &frames[1];
        manager.process_frame(*id, &data[..*len]).unwrap();

        // New BAM restarts session
        let (id, data, len) = &frames[0];
        manager.process_frame(*id, &data[..*len]).unwrap();

        // Complete the retransmission
        let (id, data, len) = &frames[1];
        assert!(matches!(
            manager.process_frame(*id, &data[..*len]).unwrap(),
            IsoTpAction::None
        ));
        let (id, data, len) = &frames[2];
        match manager.process_frame(*id, &data[..*len]).unwrap() {
            IsoTpAction::Complete(complete) => assert_eq!(complete.data, original),
            other => panic!("Expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn large_iso_tp_message() {
        let target_pgn = Pgn::new(126998);
        let source = 5;
        // 255 packets * 7 bytes = 1785 bytes (maximum)
        let original: Vec<u8> = (0..1785u16).map(|i| (i & 0xFF) as u8).collect();

        let frames: Vec<_> = IsoTpBamFragmenter::new(source, target_pgn, &original).collect();
        assert_eq!(frames.len(), 256); // 1 BAM + 255 data

        let mut manager = IsoTpManager::new();

        // Send BAM
        let (id, data, len) = &frames[0];
        manager.process_frame(*id, &data[..*len]).unwrap();

        // Send data frames in shuffled order (reverse groups of 10)
        let data_frames: Vec<_> = frames[1..].to_vec();
        let mut indices: Vec<usize> = (0..data_frames.len()).collect();
        // Reverse in chunks to create out-of-order pattern
        for chunk in indices.chunks_mut(10) {
            chunk.reverse();
        }

        let mut result = None;
        for i in indices {
            let (id, data, len) = &data_frames[i];
            if let IsoTpAction::Complete(complete) =
                manager.process_frame(*id, &data[..*len]).unwrap()
            {
                result = Some(complete);
            }
        }

        let complete = result.expect("Should reassemble 1785 bytes");
        assert_eq!(complete.pgn, target_pgn);
        assert_eq!(complete.data, original);
    }

    #[test]
    fn session_slots_exhausted() {
        let mut manager = IsoTpManager::new();
        let cm_pgn = Pgn::ISO_TP_CONNECTION_MGMT;

        // Fill all 8 session slots
        for i in 0..MAX_RX_SESSIONS {
            let source = i as u8;
            let target_pgn = Pgn::new(65240 + i as u32);
            let pgn_raw = target_pgn.as_u32();
            let cm_id = CanId::new(6, cm_pgn, 0xFF, source);
            let bam = [
                cm::BAM,
                14,
                0,
                2,
                0xFF,
                (pgn_raw & 0xFF) as u8,
                ((pgn_raw >> 8) & 0xFF) as u8,
                ((pgn_raw >> 16) & 0xFF) as u8,
            ];
            assert!(matches!(
                manager.process_frame(cm_id, &bam).unwrap(),
                IsoTpAction::None
            ));
        }

        // 9th session should fail with BufferFull
        let target_pgn = Pgn::new(70000);
        let pgn_raw = target_pgn.as_u32();
        let cm_id = CanId::new(6, cm_pgn, 0xFF, 99);
        let bam = [
            cm::BAM,
            14,
            0,
            2,
            0xFF,
            (pgn_raw & 0xFF) as u8,
            ((pgn_raw >> 8) & 0xFF) as u8,
            ((pgn_raw >> 16) & 0xFF) as u8,
        ];
        let result = manager.process_frame(cm_id, &bam);
        assert!(matches!(result, Err(IsoTpError::BufferFull)));
    }
}
