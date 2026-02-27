use std::io;

use nmea2000_pgns::DecodedMessage;
use nmea2000_types::{
    CanId, Error, FastPacketFragmenter, FastPacketReassembler, IsoTpAction, IsoTpBamFragmenter,
    IsoTpManager, PacketType, Pgn, PgnMessage, RawMessage,
};

/// Abstraction over a CAN bus transport.
///
/// Implement this trait to use [`N2kBus`] with any CAN interface.
pub trait CanTransport {
    /// Send a single CAN frame.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the frame cannot be sent.
    fn send(&self, id: CanId, data: &[u8]) -> io::Result<()>;

    /// Receive a single CAN frame (blocking).
    ///
    /// Should skip non-extended CAN frames and only return 29-bit extended frames.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if reading from the transport fails.
    fn receive(&self) -> io::Result<(CanId, Vec<u8>)>;

    /// Try to receive a CAN frame without blocking.
    ///
    /// Returns `Ok(None)` if no frame is available.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if reading from the transport fails.
    fn try_receive(&self) -> io::Result<Option<(CanId, Vec<u8>)>>;
}

/// High-level NMEA 2000 bus interface.
///
/// Wraps a CAN transport with automatic Fast Packet and ISO TP
/// reassembly/fragmentation.
pub struct N2kBus<T: CanTransport> {
    transport: T,
    fast_packet: FastPacketReassembler,
    iso_tp: IsoTpManager,
    seq_counter: u8,
}

#[cfg(feature = "socketcan")]
impl N2kBus<crate::transport::SocketCanTransport> {
    /// Open an NMEA 2000 bus on the given `SocketCAN` interface.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the CAN interface cannot be opened.
    pub fn open(interface: &str) -> io::Result<Self> {
        let transport = crate::transport::SocketCanTransport::open(interface)?;
        Ok(Self::new(transport))
    }

    /// Set the read timeout for blocking operations.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the timeout cannot be set.
    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> io::Result<()> {
        self.transport.set_read_timeout(timeout)
    }
}

impl<T: CanTransport> N2kBus<T> {
    /// Create a new N2k bus from a transport.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            fast_packet: FastPacketReassembler::new(),
            iso_tp: IsoTpManager::new(),
            seq_counter: 0,
        }
    }

    /// Read the next complete NMEA 2000 message.
    ///
    /// Automatically handles Fast Packet and ISO TP reassembly.
    /// Single-frame messages are returned immediately.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or protocol errors.
    pub fn read_message(&mut self) -> Result<RawMessage, Error> {
        loop {
            let (id, data) = self.transport.receive().map_err(|e| {
                Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
            })?;

            if let Some(msg) = self.process_frame(id, &data)? {
                return Ok(msg);
            }
        }
    }

    /// Try to read a complete message without blocking.
    ///
    /// Returns `Ok(None)` if no frame is available.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O failure or protocol errors.
    pub fn try_read_message(&mut self) -> Result<Option<RawMessage>, Error> {
        let Some((id, data)) = self
            .transport
            .try_receive()
            .map_err(|e| Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str())))?
        else {
            return Ok(None);
        };

        self.process_frame(id, &data)
    }

    /// Send a typed PGN message.
    ///
    /// Automatically fragments using Fast Packet if the message is larger than 8 bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if encoding or sending fails.
    pub fn send_pgn<M: PgnMessage>(
        &mut self,
        msg: &M,
        source: u8,
        priority: u8,
        destination: u8,
    ) -> Result<(), Error> {
        let mut buf = vec![0u8; msg.data_length()];
        let len = msg.encode(&mut buf)?;
        buf.truncate(len);

        self.send_raw(M::PGN, source, priority, destination, &buf)
    }

    /// Send raw data for a given PGN.
    ///
    /// Uses single frame, Fast Packet, or ISO TP based on the data length.
    ///
    /// # Errors
    ///
    /// Returns an error if sending fails.
    pub fn send_raw(
        &mut self,
        pgn: Pgn,
        source: u8,
        priority: u8,
        destination: u8,
        data: &[u8],
    ) -> Result<(), Error> {
        if data.len() <= 8 {
            // Single frame
            let id = CanId::new(priority, pgn, destination, source);
            self.transport.send(id, data).map_err(|e| {
                Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
            })?;
        } else if data.len() <= 223 {
            // Fast Packet (max 223 bytes)
            let seq = self.seq_counter;
            self.seq_counter = (self.seq_counter + 1) % 8;
            let fragmenter =
                FastPacketFragmenter::new(pgn, source, priority, destination, data, seq);
            for (id, frame_data, frame_len) in fragmenter {
                self.transport
                    .send(id, &frame_data[..frame_len])
                    .map_err(|e| {
                        Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
                    })?;
            }
        } else {
            // ISO TP BAM (> 223 bytes)
            let fragmenter = IsoTpBamFragmenter::new(source, pgn, data);
            for (id, frame_data, frame_len) in fragmenter {
                self.transport
                    .send(id, &frame_data[..frame_len])
                    .map_err(|e| {
                        Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
                    })?;
            }
        }

        Ok(())
    }

    /// Decode a raw message into a typed `DecodedMessage`.
    ///
    /// # Errors
    ///
    /// Returns an error if the PGN data cannot be decoded.
    pub fn decode(raw: &RawMessage) -> Result<DecodedMessage, Error> {
        DecodedMessage::decode(raw.pgn.as_u32(), &raw.data)
    }

    /// Access the underlying transport for low-level operations.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn process_frame(&mut self, id: CanId, data: &[u8]) -> Result<Option<RawMessage>, Error> {
        let pgn = id.pgn();
        let source = id.source_address();
        let destination = id.destination();
        let priority = id.priority();

        // Try ISO TP first (Connection Management and Data Transfer)
        if pgn == Pgn::ISO_TP_CONNECTION_MGMT || pgn == Pgn::ISO_TP_DATA_TRANSFER {
            match self.iso_tp.process_frame(id, data)? {
                IsoTpAction::None => return Ok(None),
                IsoTpAction::Complete(complete) => {
                    return Ok(Some(RawMessage {
                        pgn: complete.pgn,
                        source: complete.source,
                        destination: Some(complete.destination),
                        priority,
                        data: complete.data,
                    }));
                }
                IsoTpAction::SendCts {
                    destination: cts_dest,
                    max_packets,
                    next_seq,
                    pgn: cts_pgn,
                } => {
                    let cts_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, cts_dest, source);
                    let pgn_raw = cts_pgn.as_u32();
                    let cts_data = [
                        17, // CTS control byte
                        max_packets,
                        next_seq,
                        0xFF,
                        0xFF,
                        (pgn_raw & 0xFF) as u8,
                        ((pgn_raw >> 8) & 0xFF) as u8,
                        ((pgn_raw >> 16) & 0xFF) as u8,
                    ];
                    self.transport.send(cts_id, &cts_data).map_err(|e| {
                        Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
                    })?;
                    return Ok(None);
                }
                IsoTpAction::CompleteWithAck {
                    complete,
                    destination: ack_dest,
                    total_bytes,
                    total_packets,
                    pgn: ack_pgn,
                } => {
                    let ack_id = CanId::new(6, Pgn::ISO_TP_CONNECTION_MGMT, ack_dest, source);
                    let pgn_raw = ack_pgn.as_u32();
                    let ack_data = [
                        19, // EOM_ACK control byte
                        (total_bytes & 0xFF) as u8,
                        ((total_bytes >> 8) & 0xFF) as u8,
                        total_packets,
                        0xFF,
                        (pgn_raw & 0xFF) as u8,
                        ((pgn_raw >> 8) & 0xFF) as u8,
                        ((pgn_raw >> 16) & 0xFF) as u8,
                    ];
                    self.transport.send(ack_id, &ack_data).map_err(|e| {
                        Error::Encode(Box::leak(format!("I/O error: {e}").into_boxed_str()))
                    })?;
                    return Ok(Some(RawMessage {
                        pgn: complete.pgn,
                        source: complete.source,
                        destination: Some(complete.destination),
                        priority,
                        data: complete.data,
                    }));
                }
            }
        }

        // Check if this PGN uses Fast Packet
        if pgn.packet_type() == PacketType::FastPacket && data.len() <= 8 {
            if let Some(complete) = self.fast_packet.process_frame(id, data)? {
                return Ok(Some(RawMessage {
                    pgn: complete.pgn,
                    source: complete.source,
                    destination,
                    priority,
                    data: complete.data,
                }));
            }
            return Ok(None);
        }

        // Single frame — return directly
        Ok(Some(RawMessage {
            pgn,
            source,
            destination,
            priority,
            data: data.to_vec(),
        }))
    }
}
