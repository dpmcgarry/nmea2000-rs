/// A match criterion for variant dispatch.
///
/// Each criterion identifies a fixed position in the raw data
/// where a specific value must appear for a variant to be selected.
#[derive(Debug, Clone)]
pub struct MatchCriterion {
    /// Bit offset in the raw payload data.
    pub bit_offset: usize,
    /// Bit length of the discriminating field.
    pub bit_length: usize,
    /// The required value at this position.
    pub expected_value: i64,
}

/// Resolved PGN definition ready for code generation.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedPgn {
    pub pgn: u32,
    pub rust_name: String,
    pub module_name: String,
    pub description: String,
    pub packet_type: ResolvedPacketType,
    pub byte_length: Option<usize>,
    pub min_byte_length: Option<usize>,
    pub fields: Vec<ResolvedField>,
    pub repeating_set1: Option<RepeatingSet>,
    pub repeating_set2: Option<RepeatingSet>,
    pub is_complete: bool,
    pub is_proprietary: bool,
    /// Match criteria for runtime dispatch. Empty for single-variant PGNs.
    pub match_criteria: Vec<MatchCriterion>,
    /// Whether this is a fallback (catch-all) PGN entry.
    pub is_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedPacketType {
    Single,
    Fast,
    Iso,
}

/// A resolved field within a PGN.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedField {
    pub order: u32,
    pub rust_name: String,
    pub display_name: String,
    pub bit_offset: usize,
    pub bit_length: usize,
    pub is_variable_length: bool,
    pub kind: FieldKind,
    pub is_signed: bool,
    pub resolution: Option<f64>,
    pub offset: Option<i64>,
    pub unit: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum FieldKind {
    /// Raw unsigned/signed integer, possibly with resolution/offset.
    Number,
    /// IEEE 754 float (32-bit).
    Float,
    /// Enumeration lookup.
    Lookup {
        enum_name: String,
        rust_enum_name: String,
    },
    /// Bit-flag enumeration.
    BitLookup {
        enum_name: String,
        rust_enum_name: String,
    },
    /// Fixed-length ASCII string.
    StringFix { byte_length: usize },
    /// Length-prefixed string (null terminated).
    StringLz,
    /// Length-and-encoding-prefixed string.
    StringLau,
    /// Date (days since epoch).
    Date,
    /// Time (seconds since midnight, fractional).
    Time,
    /// Raw binary data.
    Binary,
    /// Reserved bits (filler).
    Reserved,
    /// Spare bits.
    Spare,
    /// MMSI (Maritime Mobile Service Identity).
    Mmsi,
    /// Reference to another PGN.
    PgnRef,
    /// Manufacturer code (special lookup).
    ManufacturerCode,
    /// Latitude (high precision i64 or i32).
    Latitude,
    /// Longitude (high precision i64 or i32).
    Longitude,
    /// BCD-encoded decimal (each nibble = one digit).
    Decimal,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct RepeatingSet {
    pub size: u32,
    pub start_field: u32,
    /// Order of the field containing the repetition count. `None` if derived from data length.
    pub count_field: Option<u32>,
}

/// A resolved lookup enumeration.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedEnum {
    pub name: String,
    pub rust_name: String,
    pub variants: Vec<ResolvedEnumVariant>,
    pub max_value: Option<u64>,
}

#[derive(Debug)]
pub struct ResolvedEnumVariant {
    pub rust_name: String,
    pub display_name: String,
    pub value: i64,
}

/// A resolved bit-flag enumeration (for future use).
#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedBitEnum {
    pub name: String,
    pub rust_name: String,
    pub flags: Vec<ResolvedBitFlag>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ResolvedBitFlag {
    pub rust_name: String,
    pub display_name: String,
    pub bit: u32,
}
