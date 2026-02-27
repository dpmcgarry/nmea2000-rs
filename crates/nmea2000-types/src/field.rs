/// Extract an unsigned integer of up to 64 bits from a byte slice.
///
/// Fields are stored LSB-first (little-endian at the bit level).
/// `bit_offset` is the starting bit position from byte 0, bit 0.
/// `bit_length` is the number of bits to extract (1-64).
#[inline]
#[must_use]
pub fn extract_uint(data: &[u8], bit_offset: usize, bit_length: usize) -> u64 {
    debug_assert!(bit_length > 0 && bit_length <= 64);
    debug_assert!(
        bit_offset + bit_length <= data.len() * 8,
        "bit_offset({bit_offset}) + bit_length({bit_length}) > data.len()*8({})",
        data.len() * 8
    );

    // Fast path for byte-aligned common sizes
    if bit_offset.is_multiple_of(8) {
        let byte_offset = bit_offset / 8;
        match bit_length {
            8 => return u64::from(data[byte_offset]),
            16 => {
                return u64::from(u16::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                ]));
            }
            32 => {
                return u64::from(u32::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                    data[byte_offset + 2],
                    data[byte_offset + 3],
                ]));
            }
            64 => {
                return u64::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                    data[byte_offset + 2],
                    data[byte_offset + 3],
                    data[byte_offset + 4],
                    data[byte_offset + 5],
                    data[byte_offset + 6],
                    data[byte_offset + 7],
                ]);
            }
            _ => {}
        }
    }

    // General case: extract bits across byte boundaries
    let mut result: u64 = 0;
    let mut bits_remaining = bit_length;
    let mut current_bit = bit_offset;
    let mut shift = 0;

    while bits_remaining > 0 {
        let byte_idx = current_bit / 8;
        let bit_in_byte = current_bit % 8;
        let bits_from_this_byte = (8 - bit_in_byte).min(bits_remaining);
        let mask = ((1u16 << bits_from_this_byte) - 1) as u8;
        let extracted = (data[byte_idx] >> bit_in_byte) & mask;
        result |= u64::from(extracted) << shift;
        shift += bits_from_this_byte;
        current_bit += bits_from_this_byte;
        bits_remaining -= bits_from_this_byte;
    }

    result
}

/// Extract a signed integer using two's complement sign extension.
#[inline]
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn extract_int(data: &[u8], bit_offset: usize, bit_length: usize) -> i64 {
    let raw = extract_uint(data, bit_offset, bit_length);
    if bit_length < 64 && (raw >> (bit_length - 1)) & 1 == 1 {
        raw as i64 | !((1i64 << bit_length) - 1)
    } else {
        raw as i64
    }
}

/// Insert an unsigned integer into a byte slice, LSB-first.
#[inline]
pub fn insert_uint(data: &mut [u8], bit_offset: usize, bit_length: usize, value: u64) {
    debug_assert!(bit_length > 0 && bit_length <= 64);
    debug_assert!(bit_offset + bit_length <= data.len() * 8);

    let mut bits_remaining = bit_length;
    let mut current_bit = bit_offset;
    let mut shift = 0;

    while bits_remaining > 0 {
        let byte_idx = current_bit / 8;
        let bit_in_byte = current_bit % 8;
        let bits_for_this_byte = (8 - bit_in_byte).min(bits_remaining);
        let mask = ((1u16 << bits_for_this_byte) - 1) as u8;
        let bits_to_write = ((value >> shift) as u8) & mask;
        data[byte_idx] = (data[byte_idx] & !(mask << bit_in_byte)) | (bits_to_write << bit_in_byte);
        shift += bits_for_this_byte;
        current_bit += bits_for_this_byte;
        bits_remaining -= bits_for_this_byte;
    }
}

/// Insert a signed integer (two's complement).
#[inline]
pub fn insert_int(data: &mut [u8], bit_offset: usize, bit_length: usize, value: i64) {
    insert_uint(data, bit_offset, bit_length, value as u64);
}

/// Check if an unsigned field value represents "unknown" (all bits set).
///
/// For 1-bit fields there is no room for an "unknown" sentinel — both 0 and 1
/// are valid values per the NMEA 2000 spec.  The convention only applies
/// to fields of 2 bits or more.
#[inline]
#[must_use]
pub const fn is_unknown_unsigned(raw: u64, bit_length: usize) -> bool {
    if bit_length <= 1 {
        false
    } else if bit_length >= 64 {
        raw == u64::MAX
    } else {
        raw == (1u64 << bit_length) - 1
    }
}

/// Check if a signed field value represents "unknown" (highest positive value).
#[inline]
#[must_use]
pub const fn is_unknown_signed(raw: i64, bit_length: usize) -> bool {
    if bit_length >= 64 {
        raw == i64::MAX
    } else {
        raw == (1i64 << (bit_length - 1)) - 1
    }
}

/// Parse a `STRING_LAU` field from data starting at position 0.
///
/// Format: `[length: u8] [encoding: u8] [data: length-2 bytes]`
/// - encoding `0x00` = `UTF-16LE`
/// - encoding `0x01` = UTF-8 / ASCII
///
/// Returns the decoded string, or `None` if the data is too short or decoding fails.
#[must_use]
pub fn parse_string_lau(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    let len = data[0] as usize;
    if len < 2 || data.len() < len {
        return None;
    }
    let enc = data[1];
    let payload = &data[2..len];
    if payload.is_empty() {
        return Some(String::new());
    }
    match enc {
        0x00 => {
            // UTF-16LE
            if !payload.len().is_multiple_of(2) {
                return None;
            }
            let u16s: Vec<u16> = payload
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16(&u16s).ok()
        }
        0x01 => {
            // UTF-8 / ASCII
            String::from_utf8(payload.to_vec()).ok()
        }
        _ => None,
    }
}

/// Returns the total number of bytes consumed by a `STRING_LAU` field.
///
/// Reads the length byte at `data[0]` to determine how many bytes the field
/// occupies. Returns 0 if data is empty.
#[inline]
#[must_use]
pub fn string_lau_total_len(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }
    (data[0] as usize).max(2)
}

/// Parse a `STRING_LZ` field from data starting at position 0.
///
/// Format: `[length: u8] [UTF-8 data: length bytes, possibly null-terminated]`
///
/// Returns the decoded string (without trailing null), or `None` if too short
/// or not valid UTF-8.
#[must_use]
pub fn parse_string_lz(data: &[u8]) -> Option<&str> {
    if data.is_empty() {
        return None;
    }
    let len = data[0] as usize;
    if len == 0 || data.len() < 1 + len {
        return None;
    }
    let payload = &data[1..=len];
    // Strip trailing null if present
    let trimmed = payload.strip_suffix(&[0]).unwrap_or(payload);
    core::str::from_utf8(trimmed).ok()
}

/// Returns the total number of bytes consumed by a `STRING_LZ` field.
#[inline]
#[must_use]
pub fn string_lz_total_len(data: &[u8]) -> usize {
    if data.is_empty() {
        return 0;
    }
    1 + data[0] as usize
}

/// Decode a BCD (Binary Coded Decimal) field.
///
/// Each nibble (4 bits) encodes one decimal digit (0-9).
/// Returns `None` if any nibble contains an invalid BCD value (>9)
/// or if all nibbles are 0xF (unknown marker).
#[must_use]
pub fn decode_decimal(data: &[u8], bit_offset: usize, bit_length: usize) -> Option<u64> {
    let raw = extract_uint(data, bit_offset, bit_length);

    // Check for "all ones" unknown marker
    let mask = if bit_length >= 64 {
        u64::MAX
    } else {
        (1u64 << bit_length) - 1
    };
    if raw == mask {
        return None;
    }

    let nibble_count = bit_length / 4;
    let mut result: u64 = 0;
    for i in (0..nibble_count).rev() {
        let nibble = ((raw >> (i * 4)) & 0x0F) as u8;
        if nibble > 9 {
            return None;
        }
        result = result * 10 + u64::from(nibble);
    }
    Some(result)
}

#[cfg(test)]
#[allow(clippy::unusual_byte_groupings)]
mod tests {
    use super::*;

    #[test]
    fn extract_byte_aligned_u8() {
        let data = [0x42, 0xAB, 0xCD];
        assert_eq!(extract_uint(&data, 0, 8), 0x42);
        assert_eq!(extract_uint(&data, 8, 8), 0xAB);
        assert_eq!(extract_uint(&data, 16, 8), 0xCD);
    }

    #[test]
    fn extract_byte_aligned_u16() {
        let data = [0x34, 0x12, 0x78, 0x56];
        assert_eq!(extract_uint(&data, 0, 16), 0x1234);
        assert_eq!(extract_uint(&data, 16, 16), 0x5678);
    }

    #[test]
    fn extract_byte_aligned_u32() {
        let data = [0x78, 0x56, 0x34, 0x12];
        assert_eq!(extract_uint(&data, 0, 32), 0x1234_5678);
    }

    #[test]
    fn extract_unaligned_bits() {
        let data = [0b1011_0100];
        assert_eq!(extract_uint(&data, 0, 3), 0b100);
        assert_eq!(extract_uint(&data, 2, 3), 0b101);
        assert_eq!(extract_uint(&data, 5, 3), 0b101);
    }

    #[test]
    fn extract_cross_byte_boundary() {
        let data = [0xFF, 0x00];
        assert_eq!(extract_uint(&data, 4, 8), 0x0F);
    }

    #[test]
    fn extract_signed_negative() {
        let data = [0xFE, 0xFF];
        assert_eq!(extract_int(&data, 0, 16), -2);
    }

    #[test]
    fn extract_signed_positive() {
        let data = [0x05, 0x00];
        assert_eq!(extract_int(&data, 0, 16), 5);
    }

    #[test]
    fn extract_3bit_signed() {
        let data = [0b00000_101];
        assert_eq!(extract_int(&data, 0, 3), -3);
    }

    #[test]
    fn insert_byte_aligned() {
        let mut data = [0u8; 4];
        insert_uint(&mut data, 0, 8, 0x42);
        insert_uint(&mut data, 8, 16, 0x1234);
        assert_eq!(data[0], 0x42);
        assert_eq!(data[1], 0x34);
        assert_eq!(data[2], 0x12);
    }

    #[test]
    fn insert_unaligned() {
        let mut data = [0u8; 2];
        insert_uint(&mut data, 0, 3, 0b101);
        assert_eq!(data[0] & 0x07, 0b101);
        insert_uint(&mut data, 3, 5, 0b11010);
        assert_eq!(data[0], 0b11010_101);
    }

    #[test]
    fn roundtrip_uint() {
        let mut data = [0u8; 8];
        let values = [(0, 7, 0x5Au64), (7, 13, 0x1ABC), (20, 32, 0xDEAD_BEEF)];
        for &(offset, length, value) in &values {
            insert_uint(&mut data, offset, length, value);
        }
        for &(offset, length, value) in &values {
            let mask = if length >= 64 {
                u64::MAX
            } else {
                (1u64 << length) - 1
            };
            assert_eq!(extract_uint(&data, offset, length), value & mask);
        }
    }

    #[test]
    fn roundtrip_int() {
        let mut data = [0u8; 8];
        insert_int(&mut data, 0, 16, -1234);
        assert_eq!(extract_int(&data, 0, 16), -1234);
        insert_int(&mut data, 16, 8, -42);
        assert_eq!(extract_int(&data, 16, 8), -42);
    }

    #[test]
    fn unknown_detection() {
        assert!(is_unknown_unsigned(0xFF, 8));
        assert!(is_unknown_unsigned(0xFFFF, 16));
        assert!(!is_unknown_unsigned(0xFE, 8));
        assert!(is_unknown_unsigned(0x07, 3));

        assert!(is_unknown_signed(0x7F, 8));
        assert!(is_unknown_signed(0x7FFF, 16));
        assert!(!is_unknown_signed(0x7E, 8));
    }

    #[test]
    fn parse_string_lau_utf8() {
        // encoding=0x01 (UTF-8), "hello"
        let data = [0x07, 0x01, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(parse_string_lau(&data), Some("hello".to_string()));
        assert_eq!(string_lau_total_len(&data), 7);
    }

    #[test]
    fn parse_string_lau_utf16le() {
        // encoding=0x00 (UTF-16LE), "wórld"
        let data = [
            0x0C, 0x00, 0x77, 0x00, 0xF3, 0x00, 0x72, 0x00, 0x6C, 0x00, 0x64, 0x00,
        ];
        assert_eq!(parse_string_lau(&data), Some("wórld".to_string()));
        assert_eq!(string_lau_total_len(&data), 12);
    }

    #[test]
    fn parse_string_lau_empty() {
        assert_eq!(parse_string_lau(&[]), None);
        assert_eq!(parse_string_lau(&[0x02, 0x01]), Some(String::new()));
        assert_eq!(string_lau_total_len(&[]), 0);
    }

    #[test]
    fn parse_string_lz_basic() {
        // length=6, "hello\0"
        let data = [0x06, b'h', b'e', b'l', b'l', b'o', 0x00];
        assert_eq!(parse_string_lz(&data), Some("hello"));
        assert_eq!(string_lz_total_len(&data), 7);
    }

    #[test]
    fn parse_string_lz_no_null() {
        // length=5, "hello" (no null terminator)
        let data = [0x05, b'h', b'e', b'l', b'l', b'o'];
        assert_eq!(parse_string_lz(&data), Some("hello"));
        assert_eq!(string_lz_total_len(&data), 6);
    }

    #[test]
    fn parse_string_lz_empty() {
        assert_eq!(parse_string_lz(&[]), None);
        assert_eq!(parse_string_lz(&[0x00]), None);
        assert_eq!(string_lz_total_len(&[]), 0);
    }

    #[test]
    fn decode_decimal_basic() {
        // BCD 0x1234 = digits 1,2,3,4 = 1234
        let data = [0x34, 0x12]; // little-endian: raw = 0x1234
        assert_eq!(decode_decimal(&data, 0, 16), Some(1234));
    }

    #[test]
    fn decode_decimal_single_byte() {
        // BCD 0x42 = digits 4,2 = 42
        let data = [0x42];
        assert_eq!(decode_decimal(&data, 0, 8), Some(42));
    }

    #[test]
    fn decode_decimal_unknown() {
        // All 1s = unknown
        let data = [0xFF, 0xFF];
        assert_eq!(decode_decimal(&data, 0, 16), None);
    }

    #[test]
    fn decode_decimal_invalid_nibble() {
        // 0xAB has nibbles A and B which are > 9
        let data = [0xAB];
        assert_eq!(decode_decimal(&data, 0, 8), None);
    }

    #[test]
    fn decode_decimal_zero() {
        let data = [0x00];
        assert_eq!(decode_decimal(&data, 0, 8), Some(0));
    }
}
