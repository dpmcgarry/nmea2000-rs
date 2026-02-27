use std::io;

use nmea2000_types::CanId;
use socketcan::{CanFrame, CanSocket, EmbeddedFrame, Socket};

use crate::bus::CanTransport;

/// `SocketCAN` transport for NMEA 2000 frames.
///
/// Wraps a raw CAN socket and provides conversion between
/// [`CanId`] + payload and `SocketCAN` frames.
pub struct SocketCanTransport {
    socket: CanSocket,
}

impl SocketCanTransport {
    /// Open a CAN interface by name (e.g. `"can0"`, `"vcan0"`).
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the interface cannot be opened.
    pub fn open(interface: &str) -> io::Result<Self> {
        let socket = CanSocket::open(interface)?;
        Ok(Self { socket })
    }

    /// Set the read timeout for blocking receive operations.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the timeout cannot be set.
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.socket.set_read_timeout(timeout)
    }

    fn extract_extended_id(frame: &CanFrame) -> Option<CanId> {
        match frame.id() {
            socketcan::Id::Extended(ext_id) => CanId::from_raw(ext_id.as_raw()),
            socketcan::Id::Standard(_) => None,
        }
    }
}

impl CanTransport for SocketCanTransport {
    fn send(&self, id: CanId, data: &[u8]) -> io::Result<()> {
        let ext_id = socketcan::ExtendedId::new(id.as_raw())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid 29-bit CAN ID"))?;
        let frame = CanFrame::new(ext_id, data)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "CAN frame too large"))?;
        self.socket.write_frame(&frame)?;
        Ok(())
    }

    fn receive(&self) -> io::Result<(CanId, Vec<u8>)> {
        loop {
            let frame = self.socket.read_frame()?;
            if let Some(id) = Self::extract_extended_id(&frame) {
                return Ok((id, frame.data().to_vec()));
            }
        }
    }

    fn try_receive(&self) -> io::Result<Option<(CanId, Vec<u8>)>> {
        self.socket.set_nonblocking(true)?;
        let result = self.socket.read_frame();
        self.socket.set_nonblocking(false)?;
        match result {
            Ok(frame) => {
                if let Some(id) = Self::extract_extended_id(&frame) {
                    Ok(Some((id, frame.data().to_vec())))
                } else {
                    Ok(None)
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }
}
