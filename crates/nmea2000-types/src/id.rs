use crate::Pgn;

/// A decomposed 29-bit NMEA 2000 CAN identifier.
///
/// ```text
///   [28:26] Priority        (3 bits, 0-7, lower = higher priority)
///   [25]    Reserved        (1 bit, always 0 for NMEA 2000)
///   [24]    Data Page       (1 bit)
///   [23:16] PDU Format (PF) (8 bits)
///   [15:8]  PDU Specific    (8 bits) — destination (PDU1) or group extension (PDU2)
///   [7:0]   Source Address  (8 bits)
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanId(u32);

impl CanId {
    const MAX_29BIT: u32 = (1 << 29) - 1;
    const PDU2_THRESHOLD: u8 = 240;

    /// Create from a raw 29-bit value. Returns `None` if `raw` exceeds 29 bits.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Option<Self> {
        if raw > Self::MAX_29BIT {
            None
        } else {
            Some(Self(raw))
        }
    }

    /// Create from constituent parts.
    #[must_use]
    pub const fn new(priority: u8, pgn: Pgn, destination: u8, source: u8) -> Self {
        let dp = (pgn.as_u32() >> 16) & 1;
        let pf = (pgn.as_u32() >> 8) & 0xFF;
        let ps = if pf < Self::PDU2_THRESHOLD as u32 {
            destination as u32
        } else {
            pgn.as_u32() & 0xFF
        };

        Self(
            ((priority as u32 & 0x07) << 26)
                | (dp << 24)
                | (pf << 16)
                | (ps << 8)
                | (source as u32),
        )
    }

    #[must_use]
    pub const fn as_raw(self) -> u32 {
        self.0
    }

    /// Message priority (0-7, lower = higher priority).
    #[must_use]
    pub const fn priority(self) -> u8 {
        ((self.0 >> 26) & 0x07) as u8
    }

    #[must_use]
    pub const fn data_page(self) -> bool {
        (self.0 >> 24) & 1 != 0
    }

    #[must_use]
    pub const fn pdu_format(self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    #[must_use]
    pub const fn pdu_specific(self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    #[must_use]
    pub const fn source_address(self) -> u8 {
        (self.0 & 0xFF) as u8
    }

    /// Extract the PGN.
    ///
    /// PDU1 (PF < 240): PGN = (DP << 16) | (PF << 8), PS is destination.
    /// PDU2 (PF >= 240): PGN = (DP << 16) | (PF << 8) | PS.
    #[must_use]
    pub const fn pgn(self) -> Pgn {
        let dp = if self.data_page() { 1u32 } else { 0 };
        let pf = self.pdu_format() as u32;
        let ps = self.pdu_specific() as u32;

        let pgn_raw = if pf < Self::PDU2_THRESHOLD as u32 {
            (dp << 16) | (pf << 8)
        } else {
            (dp << 16) | (pf << 8) | ps
        };

        Pgn::new(pgn_raw)
    }

    /// Destination address for PDU1 messages, `None` for PDU2 (broadcast).
    #[must_use]
    pub const fn destination(self) -> Option<u8> {
        if self.pdu_format() < Self::PDU2_THRESHOLD {
            Some(self.pdu_specific())
        } else {
            None
        }
    }

    /// Returns `true` if this is a PDU1 (addressed) message.
    #[must_use]
    pub const fn is_addressed(self) -> bool {
        self.pdu_format() < Self::PDU2_THRESHOLD
    }
}

impl core::fmt::Debug for CanId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CanId")
            .field("raw", &format_args!("{:#010x}", self.0))
            .field("priority", &self.priority())
            .field("pgn", &self.pgn())
            .field("source", &self.source_address())
            .field("destination", &self.destination())
            .finish()
    }
}

impl core::fmt::Display for CanId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "CAN[{:#010x} P{} PGN:{} Src:{}",
            self.0,
            self.priority(),
            self.pgn(),
            self.source_address()
        )?;
        if let Some(dst) = self.destination() {
            write!(f, " Dst:{dst}")?;
        }
        write!(f, "]")
    }
}

#[cfg(test)]
#[allow(clippy::unreadable_literal)]
mod tests {
    use super::*;

    #[test]
    fn parse_vessel_heading() {
        // PGN 127250: DP=1, PF=0xF1, PS=0x12. Priority 2, Source 23.
        let can_id = CanId::from_raw(0x09F1_1217).unwrap();
        assert_eq!(can_id.priority(), 2);
        assert!(can_id.data_page());
        assert_eq!(can_id.pdu_format(), 0xF1);
        assert_eq!(can_id.pdu_specific(), 0x12);
        assert_eq!(can_id.source_address(), 23);
        assert_eq!(can_id.pgn(), Pgn::new(127250));
        assert_eq!(can_id.destination(), None);
        assert!(!can_id.is_addressed());
    }

    #[test]
    fn parse_iso_request() {
        // PGN 59904: DP=0, PF=0xEA. Priority 6, Source 1, Dest 255.
        let can_id = CanId::from_raw(0x18EA_FF01).unwrap();
        assert_eq!(can_id.priority(), 6);
        assert!(!can_id.data_page());
        assert_eq!(can_id.pdu_format(), 0xEA);
        assert_eq!(can_id.source_address(), 1);
        assert_eq!(can_id.pgn(), Pgn::new(59904));
        assert_eq!(can_id.destination(), Some(0xFF));
        assert!(can_id.is_addressed());
    }

    #[test]
    fn parse_iso_address_claim() {
        // PGN 60928: DP=0, PF=0xEE. Priority 6, Source 42, Dest 255.
        let can_id = CanId::from_raw(0x18EE_FF2A).unwrap();
        assert_eq!(can_id.priority(), 6);
        assert_eq!(can_id.pgn(), Pgn::new(60928));
        assert_eq!(can_id.source_address(), 42);
        assert_eq!(can_id.destination(), Some(0xFF));
    }

    #[test]
    fn roundtrip_pdu2() {
        let pgn = Pgn::new(127250);
        let id = CanId::new(2, pgn, 0, 23);
        assert_eq!(id.pgn(), pgn);
        assert_eq!(id.priority(), 2);
        assert_eq!(id.source_address(), 23);
        assert_eq!(id.destination(), None);
    }

    #[test]
    fn roundtrip_pdu1() {
        let pgn = Pgn::new(59904);
        let id = CanId::new(6, pgn, 255, 1);
        assert_eq!(id.pgn(), pgn);
        assert_eq!(id.priority(), 6);
        assert_eq!(id.source_address(), 1);
        assert_eq!(id.destination(), Some(255));
    }

    #[test]
    fn rejects_invalid_29bit() {
        assert!(CanId::from_raw(0x2000_0000).is_none());
        assert!(CanId::from_raw(0xFFFF_FFFF).is_none());
        assert!(CanId::from_raw(0x1FFF_FFFF).is_some());
    }

    #[test]
    fn gnss_position_data() {
        // PGN 129029: DP=1, PF=0xF8, PS=0x05. Priority 3, Source 10.
        let can_id = CanId::from_raw(0x0DF8_050A).unwrap();
        assert_eq!(can_id.priority(), 3);
        assert_eq!(can_id.pgn(), Pgn::new(129029));
        assert_eq!(can_id.source_address(), 10);
        assert!(!can_id.is_addressed());
    }
}
