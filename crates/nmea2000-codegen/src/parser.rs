use serde::{Deserialize, Deserializer};
use std::path::Path;

fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    struct StringOrNumber;

    impl de::Visitor<'_> for StringOrNumber {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a string or number")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v.to_string()))
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }

    deserializer.deserialize_any(StringOrNumber)
}

/// Top-level canboat.json v2 schema.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CanboatDatabase {
    #[serde(default)]
    pub schema_version: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(rename = "PhysicalQuantities", default)]
    pub physical_quantities: Vec<PhysicalQuantity>,
    #[serde(rename = "FieldTypes", default)]
    pub field_types: Vec<FieldTypeDef>,
    #[serde(rename = "LookupEnumerations", default)]
    pub lookup_enumerations: Vec<LookupEnumeration>,
    #[serde(rename = "LookupBitEnumerations", default)]
    pub lookup_bit_enumerations: Vec<LookupBitEnumeration>,
    #[serde(rename = "LookupIndirectEnumerations", default)]
    pub lookup_indirect_enumerations: Vec<LookupIndirectEnumeration>,
    #[serde(rename = "LookupFieldTypeEnumerations", default)]
    pub lookup_field_type_enumerations: Vec<serde_json::Value>,
    #[serde(rename = "MissingEnumerations", default)]
    pub missing_enumerations: Vec<serde_json::Value>,
    #[serde(rename = "PGNs")]
    pub pgns: Vec<PgnDef>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PhysicalQuantity {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FieldTypeDef {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub encoding_type: Option<String>,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(rename = "BaseFieldType", default)]
    pub base_field_type: Option<String>,
    #[serde(default)]
    pub size: Option<u32>,
    #[serde(default)]
    pub signed: Option<bool>,
    #[serde(default)]
    pub resolution: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(rename = "PhysicalQuantity", default)]
    pub physical_quantity: Option<String>,
    #[serde(rename = "RangeMin", default)]
    pub range_min: Option<f64>,
    #[serde(rename = "RangeMax", default)]
    pub range_max: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LookupEnumeration {
    pub name: String,
    #[serde(default)]
    pub max_value: Option<u64>,
    #[serde(rename = "EnumValues")]
    pub enum_values: Vec<EnumValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnumValue {
    pub name: String,
    pub value: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LookupBitEnumeration {
    pub name: String,
    #[serde(rename = "EnumBitValues", default)]
    pub enum_bit_values: Vec<EnumBitValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EnumBitValue {
    pub name: String,
    pub bit: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LookupIndirectEnumeration {
    pub name: String,
    #[serde(rename = "EnumValues", default)]
    pub enum_values: Vec<IndirectEnumValue>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct IndirectEnumValue {
    pub name: String,
    pub value1: i64,
    pub value2: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PgnDef {
    #[serde(rename = "PGN")]
    pub pgn: u32,
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub explanation: Option<String>,
    #[serde(rename = "Type", default)]
    pub packet_type: Option<String>,
    #[serde(default)]
    pub complete: Option<bool>,
    #[serde(default)]
    pub missing: Option<Vec<String>>,
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(rename = "MinLength", default)]
    pub min_length: Option<u32>,
    #[serde(rename = "RepeatingFieldSet1Size", default)]
    pub repeating_field_set1_size: Option<u32>,
    #[serde(rename = "RepeatingFieldSet1StartField", default)]
    pub repeating_field_set1_start_field: Option<u32>,
    #[serde(rename = "RepeatingFieldSet1CountField", default)]
    pub repeating_field_set1_count_field: Option<u32>,
    #[serde(rename = "RepeatingFieldSet2Size", default)]
    pub repeating_field_set2_size: Option<u32>,
    #[serde(rename = "RepeatingFieldSet2StartField", default)]
    pub repeating_field_set2_start_field: Option<u32>,
    #[serde(rename = "RepeatingFieldSet2CountField", default)]
    pub repeating_field_set2_count_field: Option<u32>,
    #[serde(rename = "TransmissionInterval", default)]
    pub transmission_interval: Option<u32>,
    #[serde(rename = "TransmissionIrregular", default)]
    pub transmission_irregular: Option<bool>,
    #[serde(rename = "Fields", default)]
    pub fields: Option<Vec<FieldDef>>,
    #[serde(default)]
    pub fallback: Option<bool>,
    #[serde(rename = "URL", default)]
    pub url: Option<String>,
    #[serde(rename = "FieldCount", default)]
    pub field_count: Option<u32>,
    #[serde(rename = "Priority", default)]
    pub priority: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct FieldDef {
    pub order: u32,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "BitLength", default)]
    pub bit_length: Option<u32>,
    #[serde(rename = "BitOffset", default)]
    pub bit_offset: Option<u32>,
    #[serde(rename = "BitStart", default)]
    pub bit_start: Option<u32>,
    #[serde(rename = "BitLengthVariable", default)]
    pub bit_length_variable: Option<bool>,
    #[serde(rename = "FieldType", default)]
    pub field_type: Option<String>,
    #[serde(default)]
    pub resolution: Option<f64>,
    #[serde(default)]
    pub signed: Option<bool>,
    #[serde(default)]
    pub offset: Option<i64>,
    #[serde(rename = "RangeMin", default)]
    pub range_min: Option<f64>,
    #[serde(rename = "RangeMax", default)]
    pub range_max: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(rename = "PhysicalQuantity", default)]
    pub physical_quantity: Option<String>,
    #[serde(rename = "LookupEnumeration", default)]
    pub lookup_enumeration: Option<String>,
    #[serde(rename = "LookupIndirectEnumeration", default)]
    pub lookup_indirect_enumeration: Option<String>,
    #[serde(rename = "LookupBitEnumeration", default)]
    pub lookup_bit_enumeration: Option<String>,
    #[serde(rename = "LookupFieldTypeEnumeration", default)]
    pub lookup_field_type_enumeration: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub condition: Option<String>,
    #[serde(rename = "Match", default)]
    pub match_value: Option<i64>,
    #[serde(
        rename = "BitLengthField",
        default,
        deserialize_with = "deserialize_string_or_number"
    )]
    pub bit_length_field: Option<String>,
    #[serde(rename = "LookupIndirectEnumerationFieldOrder", default)]
    pub lookup_indirect_enumeration_field_order: Option<u32>,
    #[serde(rename = "PartOfPrimaryKey", default)]
    pub part_of_primary_key: Option<bool>,
}

/// Parse the canboat.json database file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the JSON is malformed.
pub fn parse_canboat_json(path: &Path) -> Result<CanboatDatabase, Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let db: CanboatDatabase = serde_json::from_str(&content)?;
    Ok(db)
}

#[cfg(test)]
#[allow(clippy::unreadable_literal)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn canboat_json_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data/canboat.json")
    }

    #[test]
    fn parse_canboat_database() {
        let db = parse_canboat_json(&canboat_json_path()).expect("Failed to parse canboat.json");
        assert!(!db.pgns.is_empty(), "Should have PGN definitions");
        assert!(
            db.pgns.len() > 100,
            "Expected 100+ PGNs, got {}",
            db.pgns.len()
        );

        // Check that well-known PGNs exist
        let has_address_claim = db.pgns.iter().any(|p| p.pgn == 60928);
        assert!(
            has_address_claim,
            "Should contain ISO Address Claim (60928)"
        );

        let has_vessel_heading = db.pgns.iter().any(|p| p.pgn == 127250);
        assert!(has_vessel_heading, "Should contain Vessel Heading (127250)");
    }

    #[test]
    fn parse_lookup_enumerations() {
        let db = parse_canboat_json(&canboat_json_path()).unwrap();
        assert!(
            !db.lookup_enumerations.is_empty(),
            "Should have lookup enumerations"
        );

        let manufacturer = db
            .lookup_enumerations
            .iter()
            .find(|e| e.name == "MANUFACTURER_CODE");
        assert!(
            manufacturer.is_some(),
            "Should have MANUFACTURER_CODE enumeration"
        );
    }

    #[test]
    fn pgn_has_fields() {
        let db = parse_canboat_json(&canboat_json_path()).unwrap();
        let vessel_heading = db.pgns.iter().find(|p| p.pgn == 127250).unwrap();
        let fields = vessel_heading.fields.as_ref().expect("Should have fields");
        assert!(!fields.is_empty(), "Vessel Heading should have fields");
    }
}
