/// A Parameter Group Number (PGN) identifying an NMEA 2000 message type.
///
/// PGNs are 18-bit values derived from the CAN ID, stored as `u32` for convenience.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pgn(u32);

impl Pgn {
    // Protocol PGNs
    pub const ISO_REQUEST: Self = Self(59904);
    pub const ISO_TP_DATA_TRANSFER: Self = Self(60160);
    pub const ISO_TP_CONNECTION_MGMT: Self = Self(60416);
    pub const ISO_ADDRESS_CLAIM: Self = Self(60928);
    pub const ISO_COMMANDED_ADDRESS: Self = Self(65240);

    // Common navigation PGNs
    pub const SYSTEM_TIME: Self = Self(126_992);
    pub const VESSEL_HEADING: Self = Self(127_250);
    pub const RATE_OF_TURN: Self = Self(127_251);
    pub const ATTITUDE: Self = Self(127_257);
    pub const SPEED_WATER_REFERENCED: Self = Self(128_259);
    pub const WATER_DEPTH: Self = Self(128_267);
    pub const POSITION_RAPID_UPDATE: Self = Self(129_025);
    pub const COG_SOG_RAPID_UPDATE: Self = Self(129_026);
    pub const GNSS_POSITION_DATA: Self = Self(129_029);
    pub const WIND_DATA: Self = Self(130_306);

    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Returns `true` if this PGN uses PDU1 format (addressed messages, PF < 240).
    #[must_use]
    pub const fn is_pdu1(self) -> bool {
        ((self.0 >> 8) & 0xFF) < 240
    }

    /// Returns `true` if this PGN uses PDU2 format (broadcast messages, PF >= 240).
    #[must_use]
    pub const fn is_pdu2(self) -> bool {
        !self.is_pdu1()
    }

    /// Returns `true` if this is a proprietary PGN.
    #[must_use]
    pub const fn is_proprietary(self) -> bool {
        matches!(
            self.0,
            61184 | 126_720 | 65280..=65535 | 130_816..=131_071
        )
    }

    /// Classify the expected packet type for this PGN.
    #[must_use]
    pub const fn packet_type(self) -> PacketType {
        if self.0 == Self::ISO_TP_DATA_TRANSFER.0 || self.0 == Self::ISO_TP_CONNECTION_MGMT.0 {
            return PacketType::IsoTp;
        }

        let dp = (self.0 >> 16) & 1;
        let pf = (self.0 >> 8) & 0xFF;

        if dp == 1 && pf >= 0xED {
            PacketType::FastPacket
        } else {
            PacketType::Single
        }
    }
}

impl core::fmt::Debug for Pgn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Pgn({} / {:#07x})", self.0, self.0)
    }
}

impl core::fmt::Display for Pgn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// How a PGN's data is transported over CAN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PacketType {
    /// Single CAN frame (up to 8 bytes).
    Single,
    /// Fast Packet protocol (9-223 bytes, split across multiple frames).
    FastPacket,
    /// ISO 11783-3 Transport Protocol (up to 1785 bytes).
    IsoTp,
}

#[cfg(test)]
#[allow(clippy::unreadable_literal)]
mod tests {
    use super::*;

    #[test]
    fn well_known_pgns() {
        assert_eq!(Pgn::ISO_REQUEST.as_u32(), 59904);
        assert_eq!(Pgn::VESSEL_HEADING.as_u32(), 127250);
        assert_eq!(Pgn::GNSS_POSITION_DATA.as_u32(), 129029);
    }

    #[test]
    fn pdu_classification() {
        assert!(Pgn::ISO_REQUEST.is_pdu1());
        assert!(Pgn::ISO_ADDRESS_CLAIM.is_pdu1());
        assert!(Pgn::VESSEL_HEADING.is_pdu2());
        assert!(Pgn::SYSTEM_TIME.is_pdu2());
    }

    #[test]
    fn proprietary_detection() {
        assert!(Pgn::new(61184).is_proprietary());
        assert!(Pgn::new(65280).is_proprietary());
        assert!(Pgn::new(65535).is_proprietary());
        assert!(Pgn::new(126720).is_proprietary());
        assert!(Pgn::new(130816).is_proprietary());
        assert!(!Pgn::VESSEL_HEADING.is_proprietary());
    }

    #[test]
    fn packet_type_classification() {
        assert_eq!(Pgn::ISO_TP_DATA_TRANSFER.packet_type(), PacketType::IsoTp);
        assert_eq!(Pgn::ISO_TP_CONNECTION_MGMT.packet_type(), PacketType::IsoTp);
        assert_eq!(Pgn::ISO_REQUEST.packet_type(), PacketType::Single);
        assert_eq!(Pgn::VESSEL_HEADING.packet_type(), PacketType::FastPacket);
        assert_eq!(
            Pgn::GNSS_POSITION_DATA.packet_type(),
            PacketType::FastPacket
        );
    }
}
