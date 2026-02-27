use crate::error::Error;
use crate::pgn::{PacketType, Pgn};

/// A raw NMEA 2000 message with header information and reassembled payload.
#[derive(Debug, Clone)]
pub struct RawMessage {
    pub pgn: Pgn,
    pub source: u8,
    pub destination: Option<u8>,
    pub priority: u8,
    pub data: Vec<u8>,
}

/// Trait for typed PGN message structs.
///
/// Implemented by generated PGN types to provide type-safe encoding and decoding.
pub trait PgnMessage: Sized {
    /// The PGN number for this message type.
    const PGN: Pgn;

    /// The packet transport type.
    const PACKET_TYPE: PacketType;

    /// Decode from raw payload bytes.
    ///
    /// # Errors
    ///
    /// Returns `Error::InsufficientData` if the payload is too short.
    fn decode(data: &[u8]) -> Result<Self, Error>;

    /// Encode into a byte buffer. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns `Error::InsufficientData` if the buffer is too small.
    fn encode(&self, buf: &mut [u8]) -> Result<usize, Error>;

    /// The expected data length in bytes.
    fn data_length(&self) -> usize;
}
