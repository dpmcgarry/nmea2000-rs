/// ISO 11783-5 NAME — 64-bit unique device identifier.
///
/// Used for address claiming on the NMEA 2000 bus. Lower numeric
/// values have higher priority and win arbitration.
///
/// # Bit layout (LSB first, 64 bits total)
///
/// | Bits  | Width | Field                     |
/// |-------|-------|---------------------------|
/// | 0–20  | 21    | Unique Number             |
/// | 21–31 | 11    | Manufacturer Code         |
/// | 32–34 | 3     | Device Instance Lower     |
/// | 35–39 | 5     | Device Instance Upper     |
/// | 40–47 | 8     | Device Function           |
/// | 48–48 | 1     | Reserved                  |
/// | 49–55 | 7     | Device Class              |
/// | 56–59 | 4     | System Instance           |
/// | 60–62 | 3     | Industry Group            |
/// | 63–63 | 1     | Arbitrary Address Capable  |
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IsoName(u64);

impl IsoName {
    /// Create from a raw 64-bit value.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// Create from an 8-byte array (little-endian, as transmitted on the bus).
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(bytes))
    }

    /// Return the raw 64-bit value.
    #[must_use]
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    /// Return as 8 bytes (little-endian).
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    /// Unique Number (bits 0–20, 21 bits).
    #[must_use]
    pub const fn unique_number(self) -> u32 {
        (self.0 & 0x001F_FFFF) as u32
    }

    /// Manufacturer Code (bits 21–31, 11 bits).
    #[must_use]
    pub const fn manufacturer_code(self) -> u16 {
        ((self.0 >> 21) & 0x7FF) as u16
    }

    /// Device Instance Lower (bits 32–34, 3 bits).
    #[must_use]
    pub const fn device_instance_lower(self) -> u8 {
        ((self.0 >> 32) & 0x07) as u8
    }

    /// Device Instance Upper (bits 35–39, 5 bits).
    #[must_use]
    pub const fn device_instance_upper(self) -> u8 {
        ((self.0 >> 35) & 0x1F) as u8
    }

    /// Device Instance (bits 32–39, 8 bits combined).
    #[must_use]
    pub const fn device_instance(self) -> u8 {
        ((self.0 >> 32) & 0xFF) as u8
    }

    /// Device Function (bits 40–47, 8 bits).
    #[must_use]
    pub const fn device_function(self) -> u8 {
        ((self.0 >> 40) & 0xFF) as u8
    }

    /// Device Class (bits 49–55, 7 bits).
    #[must_use]
    pub const fn device_class(self) -> u8 {
        ((self.0 >> 49) & 0x7F) as u8
    }

    /// System Instance (bits 56–59, 4 bits).
    #[must_use]
    pub const fn system_instance(self) -> u8 {
        ((self.0 >> 56) & 0x0F) as u8
    }

    /// Industry Group (bits 60–62, 3 bits).
    #[must_use]
    pub const fn industry_group(self) -> u8 {
        ((self.0 >> 60) & 0x07) as u8
    }

    /// Arbitrary Address Capable (bit 63).
    #[must_use]
    pub const fn arbitrary_address_capable(self) -> bool {
        (self.0 >> 63) != 0
    }
}

impl core::fmt::Debug for IsoName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IsoName")
            .field("unique_number", &self.unique_number())
            .field("manufacturer_code", &self.manufacturer_code())
            .field("device_instance", &self.device_instance())
            .field("device_function", &self.device_function())
            .field("device_class", &self.device_class())
            .field("system_instance", &self.system_instance())
            .field("industry_group", &self.industry_group())
            .field(
                "arbitrary_address_capable",
                &self.arbitrary_address_capable(),
            )
            .finish()
    }
}

/// Builder for constructing an [`IsoName`].
pub struct IsoNameBuilder {
    raw: u64,
}

impl IsoNameBuilder {
    /// Create a new builder with all fields zeroed.
    #[must_use]
    pub const fn new() -> Self {
        Self { raw: 0 }
    }

    /// Set the Unique Number (21 bits, 0–2097151).
    #[must_use]
    pub const fn unique_number(mut self, value: u32) -> Self {
        self.raw = (self.raw & !0x001F_FFFF) | ((value as u64) & 0x001F_FFFF);
        self
    }

    /// Set the Manufacturer Code (11 bits, 0–2047).
    #[must_use]
    pub const fn manufacturer_code(mut self, value: u16) -> Self {
        self.raw = (self.raw & !(0x7FF << 21)) | (((value as u64) & 0x7FF) << 21);
        self
    }

    /// Set the Device Instance (8 bits).
    #[must_use]
    pub const fn device_instance(mut self, value: u8) -> Self {
        self.raw = (self.raw & !(0xFF << 32)) | ((value as u64) << 32);
        self
    }

    /// Set the Device Function (8 bits).
    #[must_use]
    pub const fn device_function(mut self, value: u8) -> Self {
        self.raw = (self.raw & !(0xFF << 40)) | ((value as u64) << 40);
        self
    }

    /// Set the Device Class (7 bits, 0–127).
    #[must_use]
    pub const fn device_class(mut self, value: u8) -> Self {
        self.raw = (self.raw & !(0x7F << 49)) | (((value as u64) & 0x7F) << 49);
        self
    }

    /// Set the System Instance (4 bits, 0–15).
    #[must_use]
    pub const fn system_instance(mut self, value: u8) -> Self {
        self.raw = (self.raw & !(0x0F << 56)) | (((value as u64) & 0x0F) << 56);
        self
    }

    /// Set the Industry Group (3 bits, 0–7).
    #[must_use]
    pub const fn industry_group(mut self, value: u8) -> Self {
        self.raw = (self.raw & !(0x07 << 60)) | (((value as u64) & 0x07) << 60);
        self
    }

    /// Set the Arbitrary Address Capable flag (bit 63).
    #[must_use]
    pub const fn arbitrary_address_capable(mut self, capable: bool) -> Self {
        if capable {
            self.raw |= 1 << 63;
        } else {
            self.raw &= !(1 << 63);
        }
        self
    }

    /// Build the [`IsoName`].
    #[must_use]
    pub const fn build(self) -> IsoName {
        IsoName(self.raw)
    }
}

impl Default for IsoNameBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_roundtrip() {
        let name = IsoNameBuilder::new()
            .unique_number(1_088_507)
            .manufacturer_code(275)
            .device_instance(0)
            .device_function(130)
            .device_class(25)
            .system_instance(0)
            .industry_group(4)
            .arbitrary_address_capable(true)
            .build();

        assert_eq!(name.unique_number(), 1_088_507);
        assert_eq!(name.manufacturer_code(), 275);
        assert_eq!(name.device_instance(), 0);
        assert_eq!(name.device_function(), 130);
        assert_eq!(name.device_class(), 25);
        assert_eq!(name.system_instance(), 0);
        assert_eq!(name.industry_group(), 4);
        assert!(name.arbitrary_address_capable());
    }

    #[test]
    fn bytes_roundtrip() {
        let bytes = [0xFB, 0x9B, 0x70, 0x22, 0x00, 0x82, 0x32, 0xC0];
        let name = IsoName::from_bytes(bytes);
        assert_eq!(name.to_bytes(), bytes);
    }

    #[test]
    fn navico_conformance() {
        // Navico device: unique_number=1088507, manufacturer_code=275,
        // device_function=130, device_class=25, industry_group=4, aac=true
        let name = IsoName::from_bytes([0xFB, 0x9B, 0x70, 0x22, 0x00, 0x82, 0x32, 0xC0]);
        assert_eq!(name.unique_number(), 1_088_507);
        assert_eq!(name.manufacturer_code(), 275);
        assert_eq!(name.device_function(), 130);
        assert_eq!(name.device_class(), 25);
        assert_eq!(name.industry_group(), 4);
        assert!(name.arbitrary_address_capable());
    }

    #[test]
    fn ordering_lower_wins() {
        // Lower numeric value = higher priority (wins arbitration)
        let high_priority = IsoName::from_raw(100);
        let low_priority = IsoName::from_raw(200);
        assert!(high_priority < low_priority);
    }

    #[test]
    fn debug_shows_fields() {
        let name = IsoNameBuilder::new()
            .unique_number(42)
            .manufacturer_code(275)
            .build();
        let debug = format!("{name:?}");
        assert!(debug.contains("unique_number: 42"));
        assert!(debug.contains("manufacturer_code: 275"));
    }
}
