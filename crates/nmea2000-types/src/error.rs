use core::fmt;

use crate::Pgn;

/// Core error type for NMEA 2000 operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// CAN ID is not a valid 29-bit extended identifier.
    InvalidCanId(u32),
    /// PGN is not recognized in the database.
    UnknownPgn(Pgn),
    /// Data payload is shorter than expected.
    InsufficientData { expected: usize, actual: usize },
    /// Field value is out of the valid range.
    FieldOutOfRange { field: &'static str },
    /// Fast packet reassembly error.
    FastPacket(FastPacketError),
    /// ISO Transport Protocol error.
    IsoTp(IsoTpError),
    /// Encoding error.
    Encode(&'static str),
}

/// Errors specific to Fast Packet reassembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastPacketError {
    /// Received frame with unexpected sequence counter.
    SequenceMismatch { expected: u8, got: u8 },
    /// Missing frames in sequence.
    MissingFrames,
    /// Reassembly buffer full (too many concurrent sessions).
    BufferFull,
    /// Frame index out of range.
    InvalidFrameIndex(u8),
}

/// Errors specific to ISO Transport Protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsoTpError {
    /// Timeout waiting for response.
    Timeout,
    /// Connection aborted by peer.
    Abort(u8),
    /// Invalid state machine transition.
    InvalidState,
    /// All receive session slots are occupied.
    BufferFull,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCanId(id) => write!(f, "invalid 29-bit CAN ID: {id:#010x}"),
            Self::UnknownPgn(pgn) => write!(f, "unknown PGN: {}", pgn.as_u32()),
            Self::InsufficientData { expected, actual } => {
                write!(
                    f,
                    "insufficient data: expected {expected} bytes, got {actual}"
                )
            }
            Self::FieldOutOfRange { field } => write!(f, "field out of range: {field}"),
            Self::FastPacket(e) => write!(f, "fast packet error: {e}"),
            Self::IsoTp(e) => write!(f, "ISO TP error: {e}"),
            Self::Encode(msg) => write!(f, "encoding error: {msg}"),
        }
    }
}

impl fmt::Display for FastPacketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SequenceMismatch { expected, got } => {
                write!(f, "sequence mismatch: expected {expected}, got {got}")
            }
            Self::MissingFrames => write!(f, "missing frames in sequence"),
            Self::BufferFull => write!(f, "reassembly buffer full"),
            Self::InvalidFrameIndex(idx) => write!(f, "invalid frame index: {idx}"),
        }
    }
}

impl fmt::Display for IsoTpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(f, "timeout"),
            Self::Abort(reason) => write!(f, "connection aborted: {reason}"),
            Self::InvalidState => write!(f, "invalid state machine transition"),
            Self::BufferFull => write!(f, "all receive session slots occupied"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
#[cfg(feature = "std")]
impl std::error::Error for FastPacketError {}
#[cfg(feature = "std")]
impl std::error::Error for IsoTpError {}

impl From<FastPacketError> for Error {
    fn from(e: FastPacketError) -> Self {
        Self::FastPacket(e)
    }
}

impl From<IsoTpError> for Error {
    fn from(e: IsoTpError) -> Self {
        Self::IsoTp(e)
    }
}
