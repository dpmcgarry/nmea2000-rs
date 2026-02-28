use std::io::{self, Read, Write};
use std::time::Duration;

use nmea2000_types::{Pgn, RawMessage};

use crate::bus::N2kTransport;

const DLE: u8 = 0x10;
const STX: u8 = 0x02;
const ETX: u8 = 0x03;

/// N2K message command byte (NGT-1 → host).
const N2K_MSG_RECEIVED: u8 = 0x93;
/// N2K message command byte (host → NGT-1).
const N2K_MSG_SEND: u8 = 0x94;

/// Actisense NGT-1 transport for NMEA 2000 messages.
///
/// Implements the Actisense proprietary binary protocol over USB serial.
/// The NGT-1 delivers complete, reassembled PGN messages (no Fast Packet
/// or ISO TP handling needed), so this implements [`N2kTransport`] directly.
///
/// # Protocol
///
/// Frames are delimited by DLE-STX (start) and DLE-ETX (end), with DLE
/// byte-stuffing (0x10 in payload becomes 0x10 0x10). Each frame contains
/// a command byte, payload length, payload, and checksum.
pub struct ActisenseTransport {
    port: Box<dyn serialport::SerialPort>,
    read_buf: Vec<u8>,
}

impl ActisenseTransport {
    /// Open an Actisense NGT-1 on the given serial port.
    ///
    /// The NGT-1 typically appears as `/dev/ttyUSB0` on Linux or
    /// `COM3` on Windows at 115200 baud.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the serial port cannot be opened.
    pub fn open(port_path: &str) -> io::Result<Self> {
        Self::open_with_timeout(port_path, Duration::from_secs(2))
    }

    /// Open with a custom read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the serial port cannot be opened.
    pub fn open_with_timeout(port_path: &str, timeout: Duration) -> io::Result<Self> {
        let port = serialport::new(port_path, 115_200)
            .timeout(timeout)
            .open()
            .map_err(|e| io::Error::other(e.to_string()))?;

        Ok(Self {
            port,
            read_buf: Vec::with_capacity(2048),
        })
    }

    /// Set the serial port read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the timeout cannot be set.
    pub fn set_read_timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.port
            .set_timeout(timeout)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    /// Read a raw DLE-framed message from the serial port.
    ///
    /// Returns the command byte and de-stuffed payload.
    fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        // Wait for DLE STX
        loop {
            let b = self.read_byte()?;
            if b == DLE {
                let next = self.read_byte()?;
                if next == STX {
                    break;
                }
            }
        }

        // Read until DLE ETX
        let mut payload = Vec::with_capacity(256);
        loop {
            let b = self.read_byte()?;
            if b == DLE {
                let next = self.read_byte()?;
                match next {
                    ETX => break,             // End of frame
                    DLE => payload.push(DLE), // Escaped DLE
                    _ => {
                        // Unexpected byte after DLE — protocol error, skip frame
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected byte after DLE",
                        ));
                    }
                }
            } else {
                payload.push(b);
            }
        }

        // Minimum: command(1) + length(1) + checksum(1) = 3
        if payload.len() < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Actisense frame too short",
            ));
        }

        let command = payload[0];
        let length = payload[1] as usize;
        let data = &payload[2..payload.len() - 1];
        let _checksum = payload[payload.len() - 1];

        // Verify length
        if data.len() != length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Actisense length mismatch",
            ));
        }

        // Verify checksum (sum of command + length + data + checksum = 0 mod 256)
        let sum: u8 = payload.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if sum != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Actisense checksum error",
            ));
        }

        Ok((command, data.to_vec()))
    }

    /// Write a DLE-framed message to the serial port.
    fn write_frame(&mut self, command: u8, data: &[u8]) -> io::Result<()> {
        self.read_buf.clear();
        self.read_buf.push(DLE);
        self.read_buf.push(STX);

        // Command byte (with DLE stuffing)
        dle_stuff(&mut self.read_buf, command);
        // Length byte
        #[allow(clippy::cast_possible_truncation)]
        let len = data.len() as u8;
        dle_stuff(&mut self.read_buf, len);
        // Data (with DLE stuffing)
        for &b in data {
            dle_stuff(&mut self.read_buf, b);
        }
        // Checksum: -(command + length + sum(data)) mod 256
        let checksum = compute_checksum(command, data);
        dle_stuff(&mut self.read_buf, checksum);

        self.read_buf.push(DLE);
        self.read_buf.push(ETX);

        self.port.write_all(&self.read_buf)?;
        self.port.flush()
    }

    fn read_byte(&mut self) -> io::Result<u8> {
        let mut buf = [0u8; 1];
        self.port.read_exact(&mut buf)?;
        Ok(buf[0])
    }
}

impl N2kTransport for ActisenseTransport {
    fn send_message(&mut self, msg: &RawMessage) -> io::Result<()> {
        // N2K send payload: [priority, pgn_lo, pgn_mid, pgn_hi, dst, src, 0,0,0,0, len, data...]
        let pgn_raw = msg.pgn.as_u32();
        #[allow(clippy::cast_possible_truncation)]
        let mut payload = Vec::with_capacity(11 + msg.data.len());
        payload.push(msg.priority);
        payload.push((pgn_raw & 0xFF) as u8);
        payload.push(((pgn_raw >> 8) & 0xFF) as u8);
        payload.push(((pgn_raw >> 16) & 0xFF) as u8);
        payload.push(msg.destination.unwrap_or(0xFF));
        payload.push(msg.source);
        // 4 bytes timestamp (zeros for sending)
        payload.extend_from_slice(&[0, 0, 0, 0]);
        payload.push(msg.data.len() as u8);
        payload.extend_from_slice(&msg.data);

        self.write_frame(N2K_MSG_SEND, &payload)
    }

    fn receive_message(&mut self) -> io::Result<RawMessage> {
        loop {
            let (command, data) = self.read_frame()?;
            if command == N2K_MSG_RECEIVED
                && let Some(msg) = parse_n2k_payload(&data)
            {
                return Ok(msg);
            }
            // Skip non-N2K messages (status, configuration, etc.)
        }
    }

    fn try_receive_message(&mut self) -> io::Result<Option<RawMessage>> {
        let available = self
            .port
            .bytes_to_read()
            .map_err(|e| io::Error::other(e.to_string()))?;

        if available == 0 {
            return Ok(None);
        }

        match self.read_frame() {
            Ok((command, data)) => {
                if command == N2K_MSG_RECEIVED {
                    Ok(parse_n2k_payload(&data))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Parse an N2K message payload from the NGT-1.
///
/// Format: `[priority, pgn:3LE, dst, src, timestamp:4LE, len, data...]`
fn parse_n2k_payload(payload: &[u8]) -> Option<RawMessage> {
    // Minimum: priority(1) + pgn(3) + dst(1) + src(1) + timestamp(4) + len(1) = 11
    if payload.len() < 11 {
        return None;
    }

    let priority = payload[0];
    let pgn_raw =
        u32::from(payload[1]) | (u32::from(payload[2]) << 8) | (u32::from(payload[3]) << 16);
    let destination = payload[4];
    let source = payload[5];
    // payload[6..10] = timestamp (ignored)
    let data_len = payload[10] as usize;

    if payload.len() < 11 + data_len {
        return None;
    }

    let data = payload[11..11 + data_len].to_vec();

    Some(RawMessage {
        pgn: Pgn::new(pgn_raw),
        source,
        destination: if destination == 0xFF {
            None
        } else {
            Some(destination)
        },
        priority,
        data,
    })
}

/// DLE byte-stuffing: emit DLE DLE for 0x10 in data.
fn dle_stuff(buf: &mut Vec<u8>, byte: u8) {
    buf.push(byte);
    if byte == DLE {
        buf.push(DLE);
    }
}

/// Compute checksum: -(command + length + sum(data)) mod 256
fn compute_checksum(command: u8, data: &[u8]) -> u8 {
    #[allow(clippy::cast_possible_truncation)]
    let len = data.len() as u8;
    let sum = command
        .wrapping_add(len)
        .wrapping_add(data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)));
    (!sum).wrapping_add(1) // Two's complement negation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dle_encode_no_stuffing() {
        let mut buf = Vec::new();
        dle_stuff(&mut buf, 0x42);
        assert_eq!(buf, [0x42]);
    }

    #[test]
    fn dle_encode_with_stuffing() {
        let mut buf = Vec::new();
        dle_stuff(&mut buf, DLE);
        assert_eq!(buf, [DLE, DLE]);
    }

    #[test]
    fn checksum_roundtrip() {
        let command = 0x93;
        let data = [
            0x02, 0x00, 0xF8, 0x01, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03, 0xAA, 0xBB, 0xCC,
        ];
        let checksum = compute_checksum(command, &data);

        // Verify: command + len + sum(data) + checksum = 0 mod 256
        let len = data.len() as u8;
        let total = command
            .wrapping_add(len)
            .wrapping_add(data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b)))
            .wrapping_add(checksum);
        assert_eq!(total, 0);
    }

    #[test]
    fn parse_n2k_payload_valid() {
        // priority=2, PGN=127250 (0x01F112), dst=0xFF, src=0x01
        // timestamp=0, len=8, data=[0x00, 0x9F, 0x01, 0xFF, 0x7F, 0xFF, 0x7F, 0x00]
        let payload = [
            0x02, // priority
            0x12, 0xF1, 0x01, // PGN 127250 LE
            0xFF, // destination (broadcast)
            0x01, // source
            0x00, 0x00, 0x00, 0x00, // timestamp
            0x08, // length
            0x00, 0x9F, 0x01, 0xFF, 0x7F, 0xFF, 0x7F, 0x00, // data
        ];

        let msg = parse_n2k_payload(&payload).unwrap();
        assert_eq!(msg.pgn.as_u32(), 127250);
        assert_eq!(msg.source, 0x01);
        assert_eq!(msg.priority, 0x02);
        assert!(msg.destination.is_none()); // 0xFF = broadcast
        assert_eq!(msg.data.len(), 8);
        assert_eq!(msg.data[0], 0x00);
    }

    #[test]
    fn parse_n2k_payload_addressed() {
        let payload = [
            0x06, // priority
            0x00, 0xEA, 0x00, // PGN 59904 (ISO Request) LE
            0x01, // destination
            0x02, // source
            0x00, 0x00, 0x00, 0x00, // timestamp
            0x03, // length
            0x12, 0xF1, 0x01, // data (requesting PGN 127250)
        ];

        let msg = parse_n2k_payload(&payload).unwrap();
        assert_eq!(msg.pgn.as_u32(), 59904);
        assert_eq!(msg.destination, Some(0x01));
        assert_eq!(msg.source, 0x02);
    }

    #[test]
    fn parse_n2k_payload_too_short() {
        let payload = [0x02, 0x12, 0xF1]; // Only 3 bytes
        assert!(parse_n2k_payload(&payload).is_none());
    }

    #[test]
    fn parse_n2k_payload_truncated_data() {
        // Says len=8 but only 3 data bytes
        let payload = [
            0x02, 0x12, 0xF1, 0x01, 0xFF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x08, // len=8
            0xAA, 0xBB, 0xCC, // only 3 bytes
        ];
        assert!(parse_n2k_payload(&payload).is_none());
    }
}
