use std::cell::RefCell;
use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use nmea2000_types::CanId;

use crate::bus::CanTransport;

/// SLCAN (Serial Line CAN) transport for NMEA 2000 frames.
///
/// Implements the LAWICEL SLCAN protocol over a serial port.
/// This is the ASCII-based protocol used by many USB-CAN adapters
/// (e.g., Canable in SLCAN mode, `USBtin`, `CANable` Pro).
///
/// The adapter is initialized at 250 kbps (NMEA 2000 standard baud rate)
/// when opened.
pub struct SlcanTransport {
    port: RefCell<BufReader<Box<dyn serialport::SerialPort>>>,
    line_buf: RefCell<String>,
}

impl SlcanTransport {
    /// Open an SLCAN adapter on the given serial port.
    ///
    /// Initializes the adapter: closes any existing connection, sets
    /// 250 kbps baud rate (NMEA 2000 standard), and opens the CAN channel.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the serial port cannot be opened or
    /// the adapter does not respond correctly.
    pub fn open(port_path: &str) -> io::Result<Self> {
        Self::open_with_timeout(port_path, Duration::from_secs(2))
    }

    /// Open an SLCAN adapter with a custom serial read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the serial port cannot be opened.
    pub fn open_with_timeout(port_path: &str, timeout: Duration) -> io::Result<Self> {
        let mut port = serialport::new(port_path, 115_200)
            .timeout(timeout)
            .open()
            .map_err(|e| io::Error::other(e.to_string()))?;

        // Close any existing connection, then configure for NMEA 2000
        // C = close, S6 = 250kbps, O = open
        for cmd in &["C\r", "S6\r", "O\r"] {
            port.write_all(cmd.as_bytes())?;
            port.flush()?;
        }

        Ok(Self {
            port: RefCell::new(BufReader::new(port)),
            line_buf: RefCell::new(String::with_capacity(64)),
        })
    }

    /// Set the serial port read timeout.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the timeout cannot be set.
    pub fn set_read_timeout(&self, timeout: Duration) -> io::Result<()> {
        self.port
            .borrow_mut()
            .get_mut()
            .set_timeout(timeout)
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

impl Drop for SlcanTransport {
    fn drop(&mut self) {
        // Best-effort close of the CAN channel
        let _ = self.port.borrow_mut().get_mut().write_all(b"C\r");
    }
}

impl CanTransport for SlcanTransport {
    fn send(&self, id: CanId, data: &[u8]) -> io::Result<()> {
        // Extended frame: T{id:08X}{dlc:1X}{data:hex}\r
        let mut cmd = format!("T{:08X}{:1X}", id.as_raw(), data.len());
        for &byte in data {
            let _ = write!(cmd, "{byte:02X}");
        }
        cmd.push('\r');

        self.port.borrow_mut().get_mut().write_all(cmd.as_bytes())?;
        self.port.borrow_mut().get_mut().flush()
    }

    fn receive(&self) -> io::Result<(CanId, Vec<u8>)> {
        loop {
            let mut line_buf = self.line_buf.borrow_mut();
            line_buf.clear();
            self.port.borrow_mut().read_line(&mut line_buf)?;

            if let Some(result) = parse_extended_frame(&line_buf) {
                return Ok(result);
            }
            // Skip non-extended frames (standard frames, status, etc.)
        }
    }

    fn try_receive(&self) -> io::Result<Option<(CanId, Vec<u8>)>> {
        // Check if data is available
        let available = {
            let port = self.port.borrow();
            port.buffer().len()
                + port
                    .get_ref()
                    .bytes_to_read()
                    .map_err(|e| io::Error::other(e.to_string()))? as usize
        };

        if available == 0 {
            return Ok(None);
        }

        let mut line_buf = self.line_buf.borrow_mut();
        line_buf.clear();
        match self.port.borrow_mut().read_line(&mut line_buf) {
            Ok(_) => Ok(parse_extended_frame(&line_buf)),
            Err(e) if e.kind() == io::ErrorKind::TimedOut => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Parse an SLCAN extended frame response.
///
/// Format: `T{id:08X}{dlc:1X}{data:hex}\r`
///
/// Returns `None` for non-extended frames or parse errors.
fn parse_extended_frame(line: &str) -> Option<(CanId, Vec<u8>)> {
    let line = line.trim_end_matches(['\r', '\n']);

    // Extended frames start with 'T'
    if !line.starts_with('T') {
        return None;
    }

    // T + 8 hex (ID) + 1 hex (DLC) = minimum 10 chars
    if line.len() < 10 {
        return None;
    }

    let id_raw = u32::from_str_radix(&line[1..9], 16).ok()?;
    let dlc = usize::from_str_radix(&line[9..10], 16).ok()?;

    // Verify data length matches
    let data_hex = &line[10..];
    if data_hex.len() != dlc * 2 {
        return None;
    }

    let id = CanId::from_raw(id_raw)?;

    let mut data = Vec::with_capacity(dlc);
    for i in 0..dlc {
        let byte = u8::from_str_radix(&data_hex[i * 2..i * 2 + 2], 16).ok()?;
        data.push(byte);
    }

    Some((id, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extended_frame_valid() {
        // Vessel Heading PGN 127250 from source 0x01, priority 2
        // CAN ID: 0x09F11201, 8 data bytes
        let line = "T09F11201800F3A300FF7FFC00\r";
        let (id, data) = parse_extended_frame(line).unwrap();
        assert_eq!(id.as_raw(), 0x09F1_1201);
        assert_eq!(data.len(), 8);
        assert_eq!(data[0], 0x00);
    }

    #[test]
    fn parse_extended_frame_zero_dlc() {
        let line = "T09F112010\r";
        let (id, data) = parse_extended_frame(line).unwrap();
        assert_eq!(id.as_raw(), 0x09F1_1201);
        assert!(data.is_empty());
    }

    #[test]
    fn parse_standard_frame_rejected() {
        // Standard frame starts with 't'
        let line = "t1230800AABB\r";
        assert!(parse_extended_frame(line).is_none());
    }

    #[test]
    fn parse_malformed_rejected() {
        assert!(parse_extended_frame("T").is_none());
        assert!(parse_extended_frame("T0000000").is_none());
        // DLC says 8 but only 2 data bytes
        assert!(parse_extended_frame("T09F11201800AA\r").is_none());
    }

    #[test]
    fn parse_invalid_id_rejected() {
        // ID exceeds 29 bits
        let line = "TFFFFFFFF08AABBCCDD00112233\r";
        assert!(parse_extended_frame(line).is_none());
    }
}
