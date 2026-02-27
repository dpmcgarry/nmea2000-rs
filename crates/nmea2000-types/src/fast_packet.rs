use crate::error::FastPacketError;
use crate::id::CanId;
use crate::pgn::Pgn;

/// Maximum Fast Packet payload size (223 bytes).
pub const MAX_FAST_PACKET_SIZE: usize = 223;

/// Maximum concurrent reassembly sessions.
const MAX_SESSIONS: usize = 64;

/// Session key: uniquely identifies a Fast Packet transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionKey {
    source: u8,
    pgn: Pgn,
}

/// A single reassembly session.
struct Session {
    key: SessionKey,
    sequence: u8,
    total_bytes: usize,
    total_frames: u8,
    received_mask: u32,
    buffer: [u8; MAX_FAST_PACKET_SIZE],
}

/// A completed Fast Packet message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastPacketComplete {
    pub pgn: Pgn,
    pub source: u8,
    pub data: Vec<u8>,
}

const NONE_SESSION: Option<Session> = None;

/// Fast Packet reassembler.
///
/// Tracks multiple concurrent Fast Packet transfers and reassembles them into
/// complete messages.
///
/// ## Protocol
///
/// Frame 0: `[SeqCounter:3 | FrameCounter:5 = 0] [TotalBytes] [Data x 6]`
/// Frame N: `[SeqCounter:3 | FrameCounter:5 = N] [Data x 7]`
///
/// `SeqCounter` increments per complete message (wraps at 8).
/// `FrameCounter` counts frames within one message (0, 1, 2, ...).
pub struct FastPacketReassembler {
    sessions: [Option<Session>; MAX_SESSIONS],
}

impl FastPacketReassembler {
    /// Create a new reassembler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: [NONE_SESSION; MAX_SESSIONS],
        }
    }

    /// Process a single CAN frame.
    ///
    /// Returns `Ok(Some(complete))` when a Fast Packet message is fully reassembled.
    /// Returns `Ok(None)` when the frame was absorbed but more frames are needed.
    ///
    /// # Errors
    ///
    /// Returns `FastPacketError` if the frame is empty, has an invalid size,
    /// has a sequence mismatch, or if the reassembly buffer is full.
    pub fn process_frame(
        &mut self,
        id: CanId,
        data: &[u8],
    ) -> Result<Option<FastPacketComplete>, FastPacketError> {
        if data.is_empty() {
            return Err(FastPacketError::MissingFrames);
        }

        let pgn = id.pgn();
        let source = id.source_address();
        let key = SessionKey { source, pgn };

        let protocol_byte = data[0];
        let frame_counter = protocol_byte & 0x1F;
        let sequence = (protocol_byte >> 5) & 0x07;

        if frame_counter == 0 {
            self.start_session(key, sequence, data)
        } else {
            self.continue_session(key, sequence, frame_counter, data)
        }
    }

    fn start_session(
        &mut self,
        key: SessionKey,
        sequence: u8,
        data: &[u8],
    ) -> Result<Option<FastPacketComplete>, FastPacketError> {
        if data.len() < 2 {
            return Err(FastPacketError::MissingFrames);
        }

        let total_bytes = data[1] as usize;
        if total_bytes > MAX_FAST_PACKET_SIZE {
            return Err(FastPacketError::InvalidFrameIndex(data[1]));
        }

        // Calculate total frames needed: frame 0 carries 6 data bytes, others carry 7
        let remaining_after_first = total_bytes.saturating_sub(6);
        let total_frames = 1 + remaining_after_first.div_ceil(7);

        // Check if an existing session with same key and sequence exists — if so,
        // treat this as a retransmission of frame 0 (keep already-received frames).
        if self
            .find_session_idx(key)
            .and_then(|idx| self.sessions[idx].as_ref())
            .is_some_and(|s| s.sequence == sequence && s.received_mask & 1 != 0)
        {
            return Ok(None);
        }

        let idx = self.find_or_allocate_idx(key)?;

        self.sessions[idx] = Some(Session {
            key,
            sequence,
            total_bytes,
            total_frames: total_frames as u8,
            received_mask: 0,
            buffer: [0u8; MAX_FAST_PACKET_SIZE],
        });

        let session = self.sessions[idx].as_mut().unwrap();
        let payload = &data[2..];
        let copy_len = payload.len().min(total_bytes);
        session.buffer[..copy_len].copy_from_slice(&payload[..copy_len]);
        session.received_mask = 1; // frame 0 received

        if total_frames == 1 {
            let complete = FastPacketComplete {
                pgn: key.pgn,
                source: key.source,
                data: session.buffer[..total_bytes].to_vec(),
            };
            self.sessions[idx] = None;
            return Ok(Some(complete));
        }

        Ok(None)
    }

    fn continue_session(
        &mut self,
        key: SessionKey,
        sequence: u8,
        frame_counter: u8,
        data: &[u8],
    ) -> Result<Option<FastPacketComplete>, FastPacketError> {
        let Some(idx) = self.find_session_idx(key) else {
            return Ok(None); // No active session — silently ignore
        };

        let session = self.sessions[idx]
            .as_mut()
            .ok_or(FastPacketError::MissingFrames)?;

        // Sequence mismatch — ignore frame (may be from an old transfer)
        if session.sequence != sequence {
            return Ok(None);
        }

        // Frame counter out of range
        if frame_counter >= session.total_frames {
            return Ok(None);
        }

        // Already received this frame — ignore duplicate
        let frame_bit = 1u32 << frame_counter;
        if session.received_mask & frame_bit != 0 {
            return Ok(None);
        }

        // Calculate buffer offset for this frame:
        // Frame 0 puts data at buffer[0..6], frame N puts data at buffer[6 + (N-1)*7..]
        let buf_offset = 6 + (frame_counter as usize - 1) * 7;
        let payload = &data[1..];
        let remaining = session.total_bytes.saturating_sub(buf_offset);
        let copy_len = payload.len().min(remaining);
        if copy_len > 0 {
            session.buffer[buf_offset..buf_offset + copy_len].copy_from_slice(&payload[..copy_len]);
        }
        session.received_mask |= frame_bit;

        // Check if all frames received
        let expected_mask = if session.total_frames >= 32 {
            u32::MAX
        } else {
            (1u32 << session.total_frames) - 1
        };

        if session.received_mask == expected_mask {
            let total = session.total_bytes;
            let complete = FastPacketComplete {
                pgn: key.pgn,
                source: key.source,
                data: session.buffer[..total].to_vec(),
            };
            self.sessions[idx] = None;
            Ok(Some(complete))
        } else {
            Ok(None)
        }
    }

    fn find_or_allocate_idx(&self, key: SessionKey) -> Result<usize, FastPacketError> {
        let mut empty = None;
        for (i, slot) in self.sessions.iter().enumerate() {
            if let Some(session) = slot {
                if session.key == key {
                    return Ok(i);
                }
            } else if empty.is_none() {
                empty = Some(i);
            }
        }
        empty.ok_or(FastPacketError::BufferFull)
    }

    fn find_session_idx(&self, key: SessionKey) -> Option<usize> {
        self.sessions
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|session| session.key == key))
    }
}

impl Default for FastPacketReassembler {
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator that fragments a complete message into Fast Packet CAN frames.
pub struct FastPacketFragmenter<'a> {
    pgn: Pgn,
    source: u8,
    priority: u8,
    destination: u8,
    data: &'a [u8],
    sequence: u8,
    frame_index: u8,
    data_offset: usize,
}

impl<'a> FastPacketFragmenter<'a> {
    /// Create a new fragmenter for sending a Fast Packet message.
    #[must_use]
    pub fn new(
        pgn: Pgn,
        source: u8,
        priority: u8,
        destination: u8,
        data: &'a [u8],
        sequence: u8,
    ) -> Self {
        Self {
            pgn,
            source,
            priority,
            destination,
            data,
            sequence,
            frame_index: 0,
            data_offset: 0,
        }
    }
}

impl Iterator for FastPacketFragmenter<'_> {
    /// (CAN ID, frame data, frame data length)
    type Item = (CanId, [u8; 8], usize);

    fn next(&mut self) -> Option<Self::Item> {
        if self.data_offset >= self.data.len() && self.frame_index > 0 {
            return None;
        }
        if self.frame_index == 0 && self.data.is_empty() {
            return None;
        }

        let can_id = CanId::new(self.priority, self.pgn, self.destination, self.source);
        let mut frame = [0xFFu8; 8];
        let protocol_byte = (self.sequence << 5) | (self.frame_index & 0x1F);

        if self.frame_index == 0 {
            // First frame: protocol byte + length byte + up to 6 data bytes
            frame[0] = protocol_byte;
            frame[1] = self.data.len() as u8;
            let copy_len = self.data.len().min(6);
            frame[2..2 + copy_len].copy_from_slice(&self.data[..copy_len]);
            self.data_offset = copy_len;
            self.frame_index = 1;
            Some((can_id, frame, 2 + copy_len))
        } else {
            // Continuation: protocol byte + up to 7 data bytes
            frame[0] = protocol_byte;
            let remaining = self.data.len() - self.data_offset;
            let copy_len = remaining.min(7);
            frame[1..=copy_len]
                .copy_from_slice(&self.data[self.data_offset..self.data_offset + copy_len]);
            self.data_offset += copy_len;
            self.frame_index += 1;
            Some((can_id, frame, 1 + copy_len))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, clippy::collapsible_if)]
mod tests {
    use super::*;

    #[test]
    fn reassemble_basic() {
        let pgn = Pgn::new(129029);
        let source = 10;
        let _can_id = CanId::new(3, pgn, 0, source);

        // Create a 20-byte test message
        let original: Vec<u8> = (0..20).collect();

        // Fragment it
        let fragmenter = FastPacketFragmenter::new(pgn, source, 3, 0, &original, 0);
        let frames: Vec<_> = fragmenter.collect();

        // Should be 3 frames: 6 + 7 + 7 bytes
        assert_eq!(frames.len(), 3);

        // Reassemble
        let mut reassembler = FastPacketReassembler::new();
        let mut result = None;
        for (id, data, len) in &frames {
            if let Some(complete) = reassembler.process_frame(*id, &data[..*len]).unwrap() {
                result = Some(complete);
            }
        }

        let complete = result.expect("Should have reassembled");
        assert_eq!(complete.pgn, pgn);
        assert_eq!(complete.source, source);
        assert_eq!(complete.data, original);
    }

    #[test]
    fn reassemble_max_size() {
        let pgn = Pgn::new(130306);
        let source = 5;
        let original: Vec<u8> = (0..223).map(|i| (i & 0xFF) as u8).collect();

        let fragmenter = FastPacketFragmenter::new(pgn, source, 2, 0, &original, 3);
        let frames: Vec<_> = fragmenter.collect();

        // 6 + 31*7 = 6 + 217 = 223, so 32 frames
        assert_eq!(frames.len(), 32);

        let mut reassembler = FastPacketReassembler::new();
        let mut result = None;
        for (id, data, len) in &frames {
            if let Some(complete) = reassembler.process_frame(*id, &data[..*len]).unwrap() {
                result = Some(complete);
            }
        }

        let complete = result.expect("Should have reassembled 223 bytes");
        assert_eq!(complete.data, original);
    }

    #[test]
    fn concurrent_sessions() {
        let mut reassembler = FastPacketReassembler::new();

        let pgn_a = Pgn::new(129029);
        let pgn_b = Pgn::new(130306);

        let data_a: Vec<u8> = (0..15).collect();
        let data_b: Vec<u8> = (100..120).collect();

        let frames_a: Vec<_> = FastPacketFragmenter::new(pgn_a, 1, 3, 0, &data_a, 0).collect();
        let frames_b: Vec<_> = FastPacketFragmenter::new(pgn_b, 2, 3, 0, &data_b, 0).collect();

        // Interleave frames
        let mut results = Vec::new();
        let max_len = frames_a.len().max(frames_b.len());
        for i in 0..max_len {
            if let Some((id, data, len)) = frames_a.get(i) {
                if let Some(complete) = reassembler.process_frame(*id, &data[..*len]).unwrap() {
                    results.push(complete);
                }
            }
            if let Some((id, data, len)) = frames_b.get(i) {
                if let Some(complete) = reassembler.process_frame(*id, &data[..*len]).unwrap() {
                    results.push(complete);
                }
            }
        }

        assert_eq!(results.len(), 2);
        // Order depends on which completes first
        let result_a = results.iter().find(|r| r.pgn == pgn_a).unwrap();
        let result_b = results.iter().find(|r| r.pgn == pgn_b).unwrap();
        assert_eq!(result_a.data, data_a);
        assert_eq!(result_b.data, data_b);
    }

    #[test]
    fn buffer_full_error() {
        let mut reassembler = FastPacketReassembler::new();

        // Fill all 64 session slots with first frames
        for i in 0..MAX_SESSIONS {
            let pgn = Pgn::new(129000 + i as u32);
            let id = CanId::new(3, pgn, 0, 1);
            // First frame: protocol=0x00, length=20, then 6 bytes data
            let data = [0x00, 20, 0, 0, 0, 0, 0, 0];
            reassembler.process_frame(id, &data).unwrap();
        }

        // 65th should fail
        let pgn = Pgn::new(200000);
        let id = CanId::new(3, pgn, 0, 1);
        let data = [0x00, 20, 0, 0, 0, 0, 0, 0];
        let result = reassembler.process_frame(id, &data);
        assert_eq!(result, Err(FastPacketError::BufferFull));
    }

    // ─── recombine-frames.in tests ─────────────────────────────────────

    /// Parse a line from recombine-frames.in into (CanId, data bytes).
    fn parse_recombine_line(line: &str) -> Option<(CanId, Vec<u8>)> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 7 {
            return None;
        }
        let prio: u8 = parts[1].parse().ok()?;
        let pgn_num: u32 = parts[2].parse().ok()?;
        let src: u8 = parts[3].parse().ok()?;
        let dst: u8 = parts[4].parse().ok()?;
        let _dlc: usize = parts[5].parse().ok()?;
        let data: Vec<u8> = parts[6..]
            .iter()
            .filter_map(|s| u8::from_str_radix(s.trim(), 16).ok())
            .collect();
        let pgn = Pgn::new(pgn_num);
        let id = CanId::new(prio, pgn, dst, src);
        Some((id, data))
    }

    /// Feed a block of lines through a reassembler and collect completed messages.
    fn reassemble_block(lines: &[&str]) -> Vec<FastPacketComplete> {
        let mut reassembler = FastPacketReassembler::new();
        let mut results = Vec::new();
        for line in lines {
            if let Some((id, data)) = parse_recombine_line(line) {
                // Single-frame PGNs (dlc=8, no fast-packet protocol) pass through
                // if the PGN is not a fast-packet PGN. For this test, 130311 is
                // single-frame. We detect by checking if it looks like a fast-packet frame.
                if let Ok(Some(complete)) = reassembler.process_frame(id, &data) {
                    results.push(complete);
                }
            }
        }
        results
    }

    const RECOMBINE_INPUT: &str = include_str!("../../../data/conformance/recombine-frames.in");

    /// Scenario 1: In-order frames with a single-frame PGN interspersed.
    /// Should reassemble 1 fast-packet PGN (129029).
    /// The single-frame PGN (130311) is not a fast packet and won't be
    /// reassembled through the fast-packet reassembler.
    #[test]
    fn recombine_scenario1_in_order_with_single_frame() {
        let lines: Vec<&str> = RECOMBINE_INPUT.lines().collect();
        // Scenario 1: lines 3-10 (0-indexed: 2-9)
        let block: Vec<&str> = lines[2..10].to_vec();
        let results = reassemble_block(&block);
        // 129029 should be reassembled
        assert!(
            results.iter().any(|r| r.pgn == Pgn::new(129029)),
            "Should reassemble PGN 129029; got {:?}",
            results.iter().map(|r| r.pgn).collect::<Vec<_>>()
        );
    }

    /// Scenario 2: Two interleaved fast-packet PGNs from same source.
    /// Should reassemble both PGN 129029 and PGN 126720.
    #[test]
    fn recombine_scenario2_interleaved_pgns() {
        let lines: Vec<&str> = RECOMBINE_INPUT.lines().collect();
        // Scenario 2: lines 14-29 (0-indexed: 13-28)
        let block: Vec<&str> = lines[13..29].to_vec();
        let results = reassemble_block(&block);
        assert!(
            results.iter().any(|r| r.pgn == Pgn::new(129029)),
            "Should reassemble PGN 129029"
        );
        assert!(
            results.iter().any(|r| r.pgn == Pgn::new(126720)),
            "Should reassemble PGN 126720"
        );
    }

    /// Scenario 3: Out-of-order frames.
    /// Should reassemble PGN 129029 (frames arrive: 0,2,6,3,4,5,1).
    #[test]
    fn recombine_scenario3_out_of_order() {
        let lines: Vec<&str> = RECOMBINE_INPUT.lines().collect();
        // Scenario 3: lines 33-39 (0-indexed: 32-38)
        let block: Vec<&str> = lines[32..39].to_vec();
        let results = reassemble_block(&block);
        assert_eq!(results.len(), 1, "Should reassemble exactly 1 PGN");
        assert_eq!(results[0].pgn, Pgn::new(129029));
        // Verify it matches the in-order result
        let in_order_block: Vec<&str> = lines[2..10]
            .iter()
            .filter(|l| !l.contains("130311"))
            .copied()
            .collect();
        let in_order = reassemble_block(&in_order_block);
        assert_eq!(
            results[0].data, in_order[0].data,
            "Out-of-order data should match in-order"
        );
    }

    /// Scenario 4: Incomplete first attempt, then successful retry.
    /// First attempt missing frame 1, second attempt out-of-order but complete.
    #[test]
    fn recombine_scenario4_incomplete_retry() {
        let lines: Vec<&str> = RECOMBINE_INPUT.lines().collect();
        // Scenario 4: lines 43-55 (0-indexed: 42-54)
        let block: Vec<&str> = lines[42..55].to_vec();
        let results = reassemble_block(&block);
        assert_eq!(
            results.len(),
            1,
            "Should reassemble exactly 1 PGN from retry"
        );
        assert_eq!(results[0].pgn, Pgn::new(129029));
    }

    /// Scenario 5: 32-frame fast packet (maximum).
    /// PGN 130823, 32 frames from source 27.
    #[test]
    fn recombine_scenario5_32_frame_max() {
        let lines: Vec<&str> = RECOMBINE_INPUT.lines().collect();
        // Scenario 5: lines 59-90 (0-indexed: 58-90)
        let block: Vec<&str> = lines[58..].to_vec();
        let results = reassemble_block(&block);
        assert_eq!(results.len(), 1, "Should reassemble exactly 1 PGN");
        assert_eq!(results[0].pgn, Pgn::new(130823));
        assert_eq!(results[0].source, 27);
        // 32 frames: 6 + 31*7 = 223 bytes
        assert_eq!(results[0].data.len(), 223);
    }
}
