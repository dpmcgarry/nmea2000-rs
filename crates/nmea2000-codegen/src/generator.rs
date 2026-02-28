use std::collections::HashMap;
use std::path::Path;

use heck::{ToShoutySnakeCase, ToSnakeCase, ToUpperCamelCase};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::model::{
    FieldKind, MatchCriterion, RepeatingSet, ResolvedEnum, ResolvedEnumVariant, ResolvedField,
    ResolvedPacketType, ResolvedPgn,
};
use crate::parser::{CanboatDatabase, FieldDef, PgnDef};

/// An item in the dynamic offset computation chain.
/// Used to generate runtime code that scans past variable-length fields.
enum DynOffsetItem {
    /// A variable-length string field — skip using its `total_len` function.
    VarField(FieldKind),
    /// A fixed-length field — skip a known number of bytes.
    FixedBytes(usize),
}

/// Generate token stream steps to skip past a series of dynamic offset items.
///
/// Assumes `offset: usize` and `data: &[u8]` are in scope.
/// Does NOT produce early returns — just increments `offset`.
/// Callers must check bounds after the skip chain.
fn generate_skip_steps(items: &[DynOffsetItem]) -> Vec<TokenStream> {
    items
        .iter()
        .map(|item| match item {
            DynOffsetItem::VarField(FieldKind::StringLau) => quote! {
                if offset < data.len() {
                    offset += nmea2000_types::field::string_lau_total_len(&data[offset..]);
                }
            },
            DynOffsetItem::VarField(FieldKind::StringLz) => quote! {
                if offset < data.len() {
                    offset += nmea2000_types::field::string_lz_total_len(&data[offset..]);
                }
            },
            DynOffsetItem::VarField(_) => quote! {},
            DynOffsetItem::FixedBytes(n) => {
                let n = *n;
                quote! { offset += #n; }
            }
        })
        .collect()
}

/// Resolve the parsed canboat database into code-generation-ready model.
fn resolve_pgn(pgn_def: &PgnDef) -> ResolvedPgn {
    let is_fallback = pgn_def.fallback == Some(true);

    let rust_name = pgn_def.id.to_upper_camel_case();
    let module_name = pgn_def.id.to_snake_case();
    let description = pgn_def
        .description
        .clone()
        .unwrap_or_else(|| pgn_def.id.clone());

    let packet_type = match pgn_def.packet_type.as_deref() {
        Some("Fast") => ResolvedPacketType::Fast,
        Some("ISO") => ResolvedPacketType::Iso,
        _ => ResolvedPacketType::Single,
    };

    let byte_length = pgn_def.length.map(|l| l as usize);
    let min_byte_length = pgn_def.min_length.map(|l| l as usize);

    let is_complete = pgn_def.complete.unwrap_or(false);
    let is_proprietary = matches!(
        pgn_def.pgn,
        61184 | 126_720 | 65280..=65535 | 130_816..=131_071
    );

    let fields = if let Some(ref field_defs) = pgn_def.fields {
        resolve_fields(field_defs)
    } else {
        Vec::new()
    };

    // Extract match criteria from fields with Match values
    let match_criteria = extract_match_criteria(pgn_def, &fields);

    let repeating_set1 = match (
        pgn_def.repeating_field_set1_size,
        pgn_def.repeating_field_set1_start_field,
    ) {
        (Some(size), Some(start)) => Some(RepeatingSet {
            size,
            start_field: start,
            count_field: pgn_def.repeating_field_set1_count_field,
        }),
        _ => None,
    };

    let repeating_set2 = match (
        pgn_def.repeating_field_set2_size,
        pgn_def.repeating_field_set2_start_field,
    ) {
        (Some(size), Some(start)) => Some(RepeatingSet {
            size,
            start_field: start,
            count_field: pgn_def.repeating_field_set2_count_field,
        }),
        _ => None,
    };

    ResolvedPgn {
        pgn: pgn_def.pgn,
        rust_name,
        module_name,
        description,
        packet_type,
        byte_length,
        min_byte_length,
        fields,
        repeating_set1,
        repeating_set2,
        is_complete,
        is_proprietary,
        match_criteria,
        is_fallback,
    }
}

/// Extract match criteria by pairing `FieldDef.match_value` with resolved field offsets.
fn extract_match_criteria(
    pgn_def: &PgnDef,
    resolved_fields: &[ResolvedField],
) -> Vec<MatchCriterion> {
    let Some(ref field_defs) = pgn_def.fields else {
        return Vec::new();
    };

    field_defs
        .iter()
        .filter_map(|fd| {
            let match_value = fd.match_value?;
            // Find the corresponding resolved field by order number
            let resolved = resolved_fields.iter().find(|rf| rf.order == fd.order)?;
            Some(MatchCriterion {
                bit_offset: resolved.bit_offset,
                bit_length: resolved.bit_length,
                expected_value: match_value,
            })
        })
        .collect()
}

fn resolve_fields(fields: &[FieldDef]) -> Vec<ResolvedField> {
    let mut running_bit_offset: usize = 0;
    fields
        .iter()
        .map(|f| {
            let rust_name = sanitize_rust_ident(&f.id.to_snake_case());
            let display_name = f.name.clone().unwrap_or_else(|| f.id.clone());
            let bit_offset = f.bit_offset.map_or(running_bit_offset, |o| o as usize);
            let bit_length = f.bit_length.unwrap_or(8) as usize;
            let is_variable_length = f.bit_length_variable.unwrap_or(false);
            let is_signed = f.signed.unwrap_or(false);
            let resolution = f.resolution;
            let offset = f.offset;

            // Advance running offset for non-variable-length fields
            if !is_variable_length {
                running_bit_offset = bit_offset + bit_length;
            }

            let kind = classify_field_kind(f);

            ResolvedField {
                order: f.order,
                rust_name,
                display_name,
                bit_offset,
                bit_length,
                is_variable_length,
                kind,
                is_signed,
                resolution,
                offset,
                unit: f.unit.clone(),
                description: f.description.clone(),
            }
        })
        .collect()
}

fn classify_field_kind(f: &FieldDef) -> FieldKind {
    let ft = f.field_type.as_deref().unwrap_or("");

    // Check for lookup enumerations first
    if f.lookup_enumeration.is_some() {
        let enum_name = f.lookup_enumeration.clone().unwrap();
        let rust_enum_name = enum_name.to_upper_camel_case();
        return FieldKind::Lookup {
            enum_name,
            rust_enum_name,
        };
    }
    if f.lookup_bit_enumeration.is_some() {
        let enum_name = f.lookup_bit_enumeration.clone().unwrap();
        let rust_enum_name = enum_name.to_upper_camel_case();
        return FieldKind::BitLookup {
            enum_name,
            rust_enum_name,
        };
    }

    match ft {
        "RESERVED" | "Reserved" => FieldKind::Reserved,
        "SPARE" | "Spare" => FieldKind::Spare,
        "BINARY" | "Binary data" => FieldKind::Binary,
        "MMSI" => FieldKind::Mmsi,
        "STRING_FIX" => FieldKind::StringFix {
            byte_length: (f.bit_length.unwrap_or(0) / 8) as usize,
        },
        "STRING_LZ" => FieldKind::StringLz,
        "STRING_LAU" => FieldKind::StringLau,
        "DATE" => FieldKind::Date,
        "TIME" => FieldKind::Time,
        "FLOAT" | "IEEE Float" => FieldKind::Float,
        "PGN" => FieldKind::PgnRef,
        s if s.contains("LATITUDE") || s.contains("Latitude") => FieldKind::Latitude,
        s if s.contains("LONGITUDE") || s.contains("Longitude") => FieldKind::Longitude,
        s if s.contains("Manufacturer code") => FieldKind::ManufacturerCode,
        "DECIMAL" => FieldKind::Decimal,
        s if s.starts_with("DYNAMIC_FIELD") => FieldKind::Binary,
        _ => FieldKind::Number,
    }
}

fn sanitize_rust_ident(name: &str) -> String {
    let name = name.replace('-', "_");
    // Rust strict + reserved keywords — prefix with r# or underscore
    match name.as_str() {
        "type" | "match" | "ref" | "mod" | "in" | "use" | "override" | "self" | "super"
        | "crate" | "fn" | "let" | "mut" | "pub" | "return" | "static" | "struct" | "trait"
        | "where" | "while" | "for" | "if" | "else" | "loop" | "break" | "continue" | "move"
        | "async" | "await" | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro"
        | "try" | "typeof" | "unsafe" | "unsized" | "virtual" | "yield" | "const" | "enum"
        | "extern" | "false" | "impl" | "as" | "true" => {
            format!("r#{name}")
        }
        _ => {
            if name.starts_with(|c: char| c.is_ascii_digit()) {
                format!("_{name}")
            } else if name.is_empty() {
                "_unnamed".to_string()
            } else {
                name
            }
        }
    }
}

fn resolve_enums(db: &CanboatDatabase) -> Vec<ResolvedEnum> {
    db.lookup_enumerations
        .iter()
        .map(|e| {
            let rust_name = e.name.to_upper_camel_case();
            let mut seen_names: HashMap<String, usize> = HashMap::new();
            let mut seen_values: std::collections::HashSet<i64> = std::collections::HashSet::new();
            let variants: Vec<ResolvedEnumVariant> = e
                .enum_values
                .iter()
                .filter_map(|v| {
                    // Skip duplicate values
                    if !seen_values.insert(v.value) {
                        return None;
                    }
                    let raw_name = v.name.to_upper_camel_case();
                    let mut variant_name = sanitize_enum_variant(&raw_name);

                    // Handle duplicate names by appending value
                    let count = seen_names.entry(variant_name.clone()).or_insert(0);
                    if *count > 0 {
                        variant_name = format!("{variant_name}{}", v.value);
                    }
                    *seen_names
                        .get_mut(&sanitize_enum_variant(&raw_name))
                        .unwrap() += 1;

                    Some(ResolvedEnumVariant {
                        rust_name: variant_name,
                        display_name: v.name.clone(),
                        value: v.value,
                    })
                })
                .collect();

            ResolvedEnum {
                name: e.name.clone(),
                rust_name,
                variants,
                max_value: e.max_value,
            }
        })
        .collect()
}

fn sanitize_enum_variant(name: &str) -> String {
    let name = name.replace(['(', ')', '/', '\\', '.', ',', '\'', '"', '-', '+'], "");
    let name = name.replace(' ', "");
    if name.is_empty() {
        return "Unknown".to_string();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("V{name}")
    } else {
        name
    }
}

fn sanitize_const_name(name: &str) -> String {
    let name = name.replace(['-', '+', '/', '\\', '.', ',', '\'', '"', '#', ' '], "_");
    let name = name.replace("__", "_");
    let name = name.trim_matches('_').to_string();
    if name.is_empty() {
        return "UNKNOWN".to_string();
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        format!("V_{name}")
    } else {
        name
    }
}

/// Generate enum code for all lookup enumerations.
fn generate_enum_code(enums: &[ResolvedEnum]) -> TokenStream {
    let enum_defs: Vec<TokenStream> = enums
        .iter()
        .filter(|e| !e.variants.is_empty())
        .map(|e| {
            let enum_name = format_ident!("{}", e.rust_name);
            let variants: Vec<TokenStream> = e
                .variants
                .iter()
                .map(|v| {
                    let variant_name = format_ident!("{}", v.rust_name);
                    let value = v.value as i32;
                    quote! { #variant_name = #value }
                })
                .collect();

            let from_raw_arms: Vec<TokenStream> = e
                .variants
                .iter()
                .map(|v| {
                    let variant_name = format_ident!("{}", v.rust_name);
                    let value = v.value;
                    quote! { #value => Some(Self::#variant_name) }
                })
                .collect();

            let display_arms: Vec<TokenStream> = e
                .variants
                .iter()
                .map(|v| {
                    let variant_name = format_ident!("{}", v.rust_name);
                    let display = &v.display_name;
                    quote! { Self::#variant_name => write!(f, #display) }
                })
                .collect();

            quote! {
                #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
                #[repr(i32)]
                pub enum #enum_name {
                    #(#variants,)*
                }

                impl #enum_name {
                    #[must_use]
                    pub fn from_raw(value: i64) -> Option<Self> {
                        match value {
                            #(#from_raw_arms,)*
                            _ => None,
                        }
                    }

                    #[must_use]
                    pub fn as_raw(self) -> i64 {
                        self as i32 as i64
                    }
                }

                impl core::fmt::Display for #enum_name {
                    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        match self {
                            #(#display_arms,)*
                        }
                    }
                }
            }
        })
        .collect();

    quote! {
        #(#enum_defs)*
    }
}

/// Generate bit-flag structs for all bit lookup enumerations.
fn generate_bit_flag_code(db: &CanboatDatabase) -> TokenStream {
    let flag_defs: Vec<TokenStream> = db
        .lookup_bit_enumerations
        .iter()
        .filter(|e| !e.enum_bit_values.is_empty())
        .map(|e| {
            let struct_name = format_ident!("{}", e.name.to_upper_camel_case());
            let doc = format!("Bit flags for `{}`.", e.name);

            let constants: Vec<TokenStream> = e
                .enum_bit_values
                .iter()
                .map(|v| {
                    let const_name =
                        format_ident!("{}", sanitize_const_name(&v.name.to_shouty_snake_case()));
                    let bit = v.bit;
                    let mask: u64 = 1 << bit;
                    let flag_doc = &v.name;
                    quote! {
                        #[doc = #flag_doc]
                        pub const #const_name: u64 = #mask;
                    }
                })
                .collect();

            let display_arms: Vec<TokenStream> = e
                .enum_bit_values
                .iter()
                .map(|v| {
                    let bit = v.bit;
                    let mask: u64 = 1 << bit;
                    let name = &v.name;
                    quote! {
                        if self.0 & #mask != 0 {
                            if !first { write!(f, ", ")?; }
                            write!(f, #name)?;
                            first = false;
                        }
                    }
                })
                .collect();

            quote! {
                #[doc = #doc]
                #[derive(Clone, Copy, PartialEq, Eq, Hash)]
                pub struct #struct_name(pub u64);

                impl #struct_name {
                    #(#constants)*

                    #[must_use]
                    pub fn from_raw(value: u64) -> Self {
                        Self(value)
                    }

                    #[must_use]
                    pub fn as_raw(self) -> u64 {
                        self.0
                    }

                    #[must_use]
                    pub fn contains(self, flag: u64) -> bool {
                        self.0 & flag != 0
                    }
                }

                impl core::fmt::Debug for #struct_name {
                    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        write!(f, "{}({:#x})", stringify!(#struct_name), self.0)
                    }
                }

                impl core::fmt::Display for #struct_name {
                    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        let mut first = true;
                        #(#display_arms)*
                        if first { write!(f, "(none)")?; }
                        let _ = first;
                        Ok(())
                    }
                }
            }
        })
        .collect();

    quote! {
        #(#flag_defs)*
    }
}

/// Generate a PGN struct with decode/encode and field accessors.
fn generate_pgn_code(pgn: &ResolvedPgn, enum_map: &HashMap<String, &ResolvedEnum>) -> TokenStream {
    let struct_name = format_ident!("{}", pgn.rust_name);
    let pgn_number = pgn.pgn;
    let description = &pgn.description;

    // Only generate typed structs for complete PGNs with known fixed length
    let byte_length = match pgn.byte_length {
        Some(len) if len <= 223 => len,
        _ => {
            // Variable-length or unknown: use Vec<u8> storage
            return generate_dynamic_pgn_code(pgn, enum_map);
        }
    };
    let byte_length_lit = byte_length;

    let packet_type_token = match pgn.packet_type {
        ResolvedPacketType::Single => quote! { nmea2000_types::PacketType::Single },
        ResolvedPacketType::Fast => quote! { nmea2000_types::PacketType::FastPacket },
        ResolvedPacketType::Iso => quote! { nmea2000_types::PacketType::IsoTp },
    };

    // Generate accessor methods for non-reserved, non-spare fields
    let accessors: Vec<TokenStream> = pgn
        .fields
        .iter()
        .filter(|f| !matches!(f.kind, FieldKind::Reserved | FieldKind::Spare))
        .map(|f| generate_field_accessor(f, enum_map))
        .collect();

    // Generate builder setter methods
    let builder_name = format_ident!("{}Builder", pgn.rust_name);
    let builder_setters: Vec<TokenStream> = pgn
        .fields
        .iter()
        .filter(|f| !matches!(f.kind, FieldKind::Reserved | FieldKind::Spare))
        .filter_map(generate_builder_setter)
        .collect();

    // Generate debug field entries for enhanced Debug impl
    let debug_fields: Vec<TokenStream> = pgn
        .fields
        .iter()
        .filter(|f| !matches!(f.kind, FieldKind::Reserved | FieldKind::Spare))
        .map(|f| {
            let field_name_str = &f.rust_name;
            let method = format_ident!("{}", f.rust_name);
            quote! { s.field(#field_name_str, &self.#method()); }
        })
        .collect();

    quote! {
        #[doc = #description]
        #[derive(Clone, PartialEq, Eq)]
        pub struct #struct_name {
            raw: [u8; #byte_length_lit],
        }

        impl #struct_name {
            /// Create a builder for constructing this message.
            #[must_use]
            pub fn builder() -> #builder_name {
                #builder_name { raw: [0xFF; #byte_length_lit] }
            }

            #(#accessors)*
        }

        #[allow(clippy::used_underscore_items)]
        impl core::fmt::Debug for #struct_name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut s = f.debug_struct(stringify!(#struct_name));
                #(#debug_fields)*
                s.finish()
            }
        }

        impl nmea2000_types::PgnMessage for #struct_name {
            const PGN: nmea2000_types::Pgn = nmea2000_types::Pgn::new(#pgn_number);
            const PACKET_TYPE: nmea2000_types::PacketType = #packet_type_token;

            fn decode(data: &[u8]) -> Result<Self, nmea2000_types::Error> {
                if data.len() < #byte_length_lit {
                    return Err(nmea2000_types::Error::InsufficientData {
                        expected: #byte_length_lit,
                        actual: data.len(),
                    });
                }
                let mut raw = [0u8; #byte_length_lit];
                raw.copy_from_slice(&data[..#byte_length_lit]);
                Ok(Self { raw })
            }

            fn encode(&self, buf: &mut [u8]) -> Result<usize, nmea2000_types::Error> {
                if buf.len() < #byte_length_lit {
                    return Err(nmea2000_types::Error::InsufficientData {
                        expected: #byte_length_lit,
                        actual: buf.len(),
                    });
                }
                buf[..#byte_length_lit].copy_from_slice(&self.raw);
                Ok(#byte_length_lit)
            }

            fn data_length(&self) -> usize {
                #byte_length_lit
            }
        }

        /// Builder for constructing a [`#struct_name`] message.
        pub struct #builder_name {
            raw: [u8; #byte_length_lit],
        }

        impl #builder_name {
            #(#builder_setters)*

            /// Build the message.
            #[must_use]
            pub fn build(self) -> #struct_name {
                #struct_name { raw: self.raw }
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
fn generate_dynamic_pgn_code(
    pgn: &ResolvedPgn,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let struct_name = format_ident!("{}", pgn.rust_name);
    let pgn_number = pgn.pgn;
    let description = &pgn.description;
    let packet_type_token = match pgn.packet_type {
        ResolvedPacketType::Single => quote! { nmea2000_types::PacketType::Single },
        ResolvedPacketType::Fast => quote! { nmea2000_types::PacketType::FastPacket },
        ResolvedPacketType::Iso => quote! { nmea2000_types::PacketType::IsoTp },
    };

    // Determine which fields are fixed (before repeating set)
    let rep_start_order = pgn.repeating_set1.as_ref().map(|rs| rs.start_field);

    // Generate accessors for fields before the repeating set.
    // Before any variable-length field: normal accessors with static offset.
    // After a variable-length field: dynamic offset computed by scanning past
    // all preceding variable/fixed fields at runtime.
    let mut accessors: Vec<TokenStream> = Vec::new();
    let mut saw_variable = false;
    let mut last_fixed_byte_end: usize = 0;
    // Skip items: instructions for computing dynamic offset at runtime.
    // Each item is either a variable-length field skip or a fixed byte skip.
    let mut skip_items: Vec<DynOffsetItem> = Vec::new();

    for f in &pgn.fields {
        // Stop at repeating set
        if rep_start_order.is_some_and(|start| f.order >= start) {
            break;
        }
        if matches!(f.kind, FieldKind::Reserved | FieldKind::Spare) {
            if f.is_variable_length {
                saw_variable = true;
                skip_items.push(DynOffsetItem::VarField(f.kind.clone()));
            } else if saw_variable {
                skip_items.push(DynOffsetItem::FixedBytes(f.bit_length.div_ceil(8)));
            } else {
                last_fixed_byte_end = (f.bit_offset + f.bit_length).div_ceil(8);
            }
            continue;
        }

        if saw_variable {
            // After a variable-length field — generate accessor with dynamic offset
            accessors.push(generate_dynoffset_accessor(
                f,
                last_fixed_byte_end,
                &skip_items,
                enum_map,
            ));
            if f.is_variable_length {
                skip_items.push(DynOffsetItem::VarField(f.kind.clone()));
            } else {
                skip_items.push(DynOffsetItem::FixedBytes(f.bit_length.div_ceil(8)));
            }
        } else if f.is_variable_length {
            saw_variable = true;
            if matches!(f.kind, FieldKind::StringLau | FieldKind::StringLz) {
                accessors.push(generate_dynoffset_accessor(
                    f,
                    last_fixed_byte_end,
                    &skip_items,
                    enum_map,
                ));
            }
            skip_items.push(DynOffsetItem::VarField(f.kind.clone()));
        } else {
            // Fixed field before any variable-length field — offset is known
            accessors.push(generate_dynamic_field_accessor(f, enum_map));
            last_fixed_byte_end = (f.bit_offset + f.bit_length).div_ceil(8);
        }
    }

    // Generate repeating set iterators
    let rep_set_code = generate_repeating_sets(pgn, enum_map);

    // Generate iterator methods on the struct
    let rep_set_methods =
        generate_repeating_set_methods(pgn, saw_variable, last_fixed_byte_end, &skip_items);

    // Generate dynamic builder
    let builder_code = generate_dynamic_builder(pgn);

    // Collect field names for Debug impl (mirrors the accessor generation logic above)
    let debug_fields: Vec<TokenStream> = {
        let mut names: Vec<&str> = Vec::new();
        let mut saw_var = false;
        for f in &pgn.fields {
            if rep_start_order.is_some_and(|start| f.order >= start) {
                break;
            }
            if matches!(f.kind, FieldKind::Reserved | FieldKind::Spare) {
                if f.is_variable_length {
                    saw_var = true;
                }
                continue;
            }
            if saw_var {
                names.push(f.rust_name.as_str());
            } else if f.is_variable_length {
                if matches!(f.kind, FieldKind::StringLau | FieldKind::StringLz) {
                    names.push(f.rust_name.as_str());
                }
                saw_var = true;
            } else {
                names.push(f.rust_name.as_str());
            }
        }
        names
            .iter()
            .map(|name| {
                let method = format_ident!("{}", name);
                let name_str = *name;
                quote! { s.field(#name_str, &self.#method()); }
            })
            .collect()
    };

    quote! {
        #[doc = #description]
        #[derive(Clone, PartialEq, Eq)]
        pub struct #struct_name {
            raw: Vec<u8>,
        }

        impl #struct_name {
            /// Access the raw bytes.
            #[must_use]
            pub fn raw(&self) -> &[u8] {
                &self.raw
            }

            #(#accessors)*

            #rep_set_methods
        }

        #[allow(clippy::used_underscore_items)]
        impl core::fmt::Debug for #struct_name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut s = f.debug_struct(stringify!(#struct_name));
                #(#debug_fields)*
                s.field("raw_len", &self.raw.len());
                s.finish()
            }
        }

        #rep_set_code

        impl nmea2000_types::PgnMessage for #struct_name {
            const PGN: nmea2000_types::Pgn = nmea2000_types::Pgn::new(#pgn_number);
            const PACKET_TYPE: nmea2000_types::PacketType = #packet_type_token;

            fn decode(data: &[u8]) -> Result<Self, nmea2000_types::Error> {
                Ok(Self { raw: data.to_vec() })
            }

            fn encode(&self, buf: &mut [u8]) -> Result<usize, nmea2000_types::Error> {
                if buf.len() < self.raw.len() {
                    return Err(nmea2000_types::Error::InsufficientData {
                        expected: self.raw.len(),
                        actual: buf.len(),
                    });
                }
                buf[..self.raw.len()].copy_from_slice(&self.raw);
                Ok(self.raw.len())
            }

            fn data_length(&self) -> usize {
                self.raw.len()
            }
        }

        #builder_code
    }
}

/// Generate a builder for a dynamic (Vec-backed) PGN.
///
/// The builder pre-allocates the fixed header, provides typed setters for fixed fields
/// and `STRING_LAU`/`STRING_LZ` fields, and `add_entry()` methods for repeating sets.
/// Count fields are patched in `build()`.
#[allow(clippy::too_many_lines)]
fn generate_dynamic_builder(pgn: &ResolvedPgn) -> TokenStream {
    let struct_name = format_ident!("{}", pgn.rust_name);
    let builder_name = format_ident!("{}Builder", pgn.rust_name);

    let rep_start_order = pgn.repeating_set1.as_ref().map(|rs| rs.start_field);

    // Compute fixed header: all non-variable fields before repeating set or first variable field
    let mut fixed_header_bits: usize = 0;
    let mut fixed_fields: Vec<&ResolvedField> = Vec::new();
    let mut string_fields: Vec<&ResolvedField> = Vec::new();

    for f in &pgn.fields {
        if rep_start_order.is_some_and(|start| f.order >= start) {
            break;
        }
        if f.is_variable_length {
            if matches!(f.kind, FieldKind::StringLau | FieldKind::StringLz) {
                string_fields.push(f);
            }
            continue;
        }
        fixed_header_bits = f.bit_offset + f.bit_length;
        if !matches!(f.kind, FieldKind::Reserved | FieldKind::Spare) {
            fixed_fields.push(f);
        }
    }

    let fixed_header_bytes = fixed_header_bits.div_ceil(8);

    // Fixed field setters (reuse existing builder setter generator)
    let fixed_setters: Vec<TokenStream> = fixed_fields
        .iter()
        .filter_map(|f| generate_builder_setter(f))
        .collect();

    // STRING_LAU / STRING_LZ setters
    let string_setters: Vec<TokenStream> = string_fields
        .iter()
        .map(|f| {
            let method_name = format_ident!("{}", f.rust_name);
            let doc = format!("Set {}", f.display_name);
            match &f.kind {
                FieldKind::StringLau => quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: &str) -> Self {
                        let bytes = value.as_bytes();
                        let total_len = (bytes.len() + 2) as u8;
                        self.raw.push(total_len);
                        self.raw.push(0x01); // UTF-8
                        self.raw.extend_from_slice(bytes);
                        self
                    }
                },
                FieldKind::StringLz => quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: &str) -> Self {
                        let bytes = value.as_bytes();
                        self.raw.extend_from_slice(bytes);
                        self.raw.push(0x00);
                        self
                    }
                },
                _ => quote! {},
            }
        })
        .collect();

    // Repeating set 1 add method + builder fields
    let mut builder_extra_fields = Vec::new();
    let mut builder_extra_inits = Vec::new();
    let mut add_methods = Vec::new();
    let mut build_patches = Vec::new();

    if let Some(ref rs) = pgn.repeating_set1 {
        let rep_fields: Vec<&ResolvedField> = pgn
            .fields
            .iter()
            .filter(|f| f.order >= rs.start_field && f.order < rs.start_field + rs.size)
            .filter(|f| !f.is_variable_length)
            .collect();

        if !rep_fields.is_empty() {
            let set_bit_size: usize = rep_fields.iter().map(|f| f.bit_length).sum();
            let set_byte_size = set_bit_size.div_ceil(8);
            let first_bit_offset = rep_fields[0].bit_offset;

            builder_extra_fields.push(quote! { set1_count: usize });
            builder_extra_inits.push(quote! { set1_count: 0 });

            let (params, inserts) = generate_entry_builder_params(&rep_fields, first_bit_offset);

            add_methods.push(quote! {
                /// Add a repeating field set entry.
                #[allow(clippy::too_many_arguments, unused_mut)]
                pub fn add_entry(mut self, #(#params),*) -> Self {
                    let mut entry = vec![0xFFu8; #set_byte_size];
                    #(#inserts)*
                    self.raw.extend_from_slice(&entry);
                    self.set1_count += 1;
                    self
                }
            });

            // Patch count field in build()
            if let Some(count_f) = rs
                .count_field
                .and_then(|co| pgn.fields.iter().find(|f| f.order == co))
            {
                let count_offset = count_f.bit_offset;
                let count_length = count_f.bit_length;
                build_patches.push(quote! {
                    nmea2000_types::field::insert_uint(
                        &mut self.raw, #count_offset, #count_length, self.set1_count as u64,
                    );
                });
            }
        }
    }

    if let Some(ref rs) = pgn.repeating_set2 {
        let rep_fields: Vec<&ResolvedField> = pgn
            .fields
            .iter()
            .filter(|f| f.order >= rs.start_field && f.order < rs.start_field + rs.size)
            .filter(|f| !f.is_variable_length)
            .collect();

        if !rep_fields.is_empty() {
            let set_bit_size: usize = rep_fields.iter().map(|f| f.bit_length).sum();
            let set_byte_size = set_bit_size.div_ceil(8);
            let first_bit_offset = rep_fields[0].bit_offset;

            builder_extra_fields.push(quote! { set2_count: usize });
            builder_extra_inits.push(quote! { set2_count: 0 });

            let (params, inserts) = generate_entry_builder_params(&rep_fields, first_bit_offset);

            add_methods.push(quote! {
                /// Add a second repeating field set entry.
                #[allow(clippy::too_many_arguments, unused_mut)]
                pub fn add_entry2(mut self, #(#params),*) -> Self {
                    let mut entry = vec![0xFFu8; #set_byte_size];
                    #(#inserts)*
                    self.raw.extend_from_slice(&entry);
                    self.set2_count += 1;
                    self
                }
            });

            if let Some(count_f) = rs
                .count_field
                .and_then(|co| pgn.fields.iter().find(|f| f.order == co))
            {
                let count_offset = count_f.bit_offset;
                let count_length = count_f.bit_length;
                build_patches.push(quote! {
                    nmea2000_types::field::insert_uint(
                        &mut self.raw, #count_offset, #count_length, self.set2_count as u64,
                    );
                });
            }
        }
    }

    let build_self_param = if build_patches.is_empty() {
        quote! { self }
    } else {
        quote! { mut self }
    };

    quote! {
        /// Builder for constructing this message.
        pub struct #builder_name {
            raw: Vec<u8>,
            #(#builder_extra_fields,)*
        }

        impl #struct_name {
            /// Create a builder for constructing this message.
            #[must_use]
            pub fn builder() -> #builder_name {
                #builder_name {
                    raw: vec![0xFFu8; #fixed_header_bytes],
                    #(#builder_extra_inits,)*
                }
            }
        }

        impl #builder_name {
            #(#fixed_setters)*
            #(#string_setters)*
            #(#add_methods)*

            /// Build the message.
            #[must_use]
            pub fn build(#build_self_param) -> #struct_name {
                #(#build_patches)*
                #struct_name { raw: self.raw }
            }
        }
    }
}

/// Generate typed parameters and insertion code for a repeating set entry builder.
///
/// Returns `(params, inserts)` where `params` are `name: Type` token streams
/// and `inserts` are `insert_uint/insert_int` statements operating on a local `entry` buffer.
#[allow(clippy::too_many_lines)]
fn generate_entry_builder_params(
    fields: &[&ResolvedField],
    first_bit_offset: usize,
) -> (Vec<TokenStream>, Vec<TokenStream>) {
    let mut params = Vec::new();
    let mut inserts = Vec::new();

    for f in fields {
        if matches!(f.kind, FieldKind::Reserved | FieldKind::Spare) {
            continue;
        }
        let param_name = format_ident!("{}", f.rust_name);
        let rel_offset = f.bit_offset.saturating_sub(first_bit_offset);
        let bit_length = f.bit_length;

        match &f.kind {
            FieldKind::Number if f.is_signed => {
                if let Some(res) = f.resolution {
                    if (res - 1.0).abs() < f64::EPSILON && f.offset.is_none() {
                        params.push(quote! { #param_name: i64 });
                        inserts.push(quote! {
                            nmea2000_types::field::insert_int(&mut entry, #rel_offset, #bit_length, #param_name);
                        });
                    } else if let Some(offset) = f.offset {
                        #[allow(clippy::cast_precision_loss)]
                        let offset_f64 = offset as f64;
                        params.push(quote! { #param_name: f64 });
                        inserts.push(quote! {
                            let raw = ((#param_name - #offset_f64) / #res) as i64;
                            nmea2000_types::field::insert_int(&mut entry, #rel_offset, #bit_length, raw);
                        });
                    } else {
                        params.push(quote! { #param_name: f64 });
                        inserts.push(quote! {
                            let raw = (#param_name / #res) as i64;
                            nmea2000_types::field::insert_int(&mut entry, #rel_offset, #bit_length, raw);
                        });
                    }
                } else {
                    params.push(quote! { #param_name: i64 });
                    inserts.push(quote! {
                        nmea2000_types::field::insert_int(&mut entry, #rel_offset, #bit_length, #param_name);
                    });
                }
            }
            FieldKind::Latitude | FieldKind::Longitude if f.is_signed => {
                let res = f.resolution.unwrap_or(1e-7);
                params.push(quote! { #param_name: f64 });
                inserts.push(quote! {
                    let raw = (#param_name / #res) as i64;
                    nmea2000_types::field::insert_int(&mut entry, #rel_offset, #bit_length, raw);
                });
            }
            FieldKind::Number
            | FieldKind::Lookup { .. }
            | FieldKind::BitLookup { .. }
            | FieldKind::Mmsi
            | FieldKind::ManufacturerCode
            | FieldKind::PgnRef => {
                if let Some(res) = f.resolution {
                    if (res - 1.0).abs() < f64::EPSILON && f.offset.is_none() {
                        params.push(quote! { #param_name: u64 });
                        inserts.push(quote! {
                            nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, #param_name);
                        });
                    } else if let Some(offset) = f.offset {
                        #[allow(clippy::cast_precision_loss)]
                        let offset_f64 = offset as f64;
                        params.push(quote! { #param_name: f64 });
                        inserts.push(quote! {
                            let raw = ((#param_name - #offset_f64) / #res) as u64;
                            nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, raw);
                        });
                    } else {
                        params.push(quote! { #param_name: f64 });
                        inserts.push(quote! {
                            let raw = (#param_name / #res) as u64;
                            nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, raw);
                        });
                    }
                } else {
                    params.push(quote! { #param_name: u64 });
                    inserts.push(quote! {
                        nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, #param_name);
                    });
                }
            }
            FieldKind::Latitude | FieldKind::Longitude => {
                let res = f.resolution.unwrap_or(1e-7);
                params.push(quote! { #param_name: f64 });
                inserts.push(quote! {
                    let raw = (#param_name / #res) as u64;
                    nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, raw);
                });
            }
            FieldKind::Date => {
                params.push(quote! { #param_name: u16 });
                inserts.push(quote! {
                    nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, #param_name as u64);
                });
            }
            FieldKind::Time => {
                let res = f.resolution.unwrap_or(0.0001);
                params.push(quote! { #param_name: f64 });
                inserts.push(quote! {
                    let raw = (#param_name / #res) as u64;
                    nmea2000_types::field::insert_uint(&mut entry, #rel_offset, #bit_length, raw);
                });
            }
            FieldKind::Float => {
                params.push(quote! { #param_name: f32 });
                inserts.push(quote! {
                    let bytes = #param_name.to_le_bytes();
                    let byte_offset = #rel_offset / 8;
                    if byte_offset + 4 <= entry.len() {
                        entry[byte_offset..byte_offset + 4].copy_from_slice(&bytes);
                    }
                });
            }
            _ => {} // Skip Binary, StringFix, StringLau, StringLz within entries
        }
    }

    (params, inserts)
}

/// Generate methods on the PGN struct that return iterators for repeating sets.
///
/// When `has_var_fields` is true, the repeating set start offset is computed at
/// runtime by scanning past variable-length fields using `skip_items`.
#[allow(unused_variables, clippy::too_many_lines)]
fn generate_repeating_set_methods(
    pgn: &ResolvedPgn,
    has_var_fields: bool,
    last_fixed_byte_end: usize,
    skip_items: &[DynOffsetItem],
) -> TokenStream {
    let mut methods = Vec::new();

    if let Some(ref rs) = pgn.repeating_set1 {
        let iter_name = format_ident!("{}ListIter", pgn.rust_name);

        // Calculate bit offset where the repeating set starts
        let rep_fields: Vec<&ResolvedField> = pgn
            .fields
            .iter()
            .filter(|f| f.order >= rs.start_field && f.order < rs.start_field + rs.size)
            .collect();
        if rep_fields.is_empty() {
            return quote! {};
        }
        let set_bit_size: usize = rep_fields.iter().map(|f| f.bit_length).sum();

        // Find the count field accessor name, if available
        let count_field = rs
            .count_field
            .and_then(|cf_order| pgn.fields.iter().find(|f| f.order == cf_order));

        // Generate offset computation — static or dynamic
        let offset_bits_expr = if has_var_fields {
            let skip_steps = generate_skip_steps(skip_items);
            quote! {
                let mut offset = #last_fixed_byte_end;
                let data = &self.raw;
                #(#skip_steps)*
                let offset_bits = offset * 8;
            }
        } else {
            let start_bit_offset = rep_fields[0].bit_offset;
            quote! { let offset_bits = #start_bit_offset; }
        };

        let iter_body = if let Some(cf) = count_field {
            let count_method = format_ident!("{}", cf.rust_name);
            quote! {
                /// Iterate over the repeating field set entries.
                #[must_use]
                pub fn list(&self) -> #iter_name<'_> {
                    let count = self.#count_method().unwrap_or(0) as usize;
                    #offset_bits_expr
                    #iter_name {
                        data: &self.raw,
                        offset_bits,
                        remaining: count,
                        set_size_bits: #set_bit_size,
                    }
                }
            }
        } else {
            // No count field — derive count from remaining data
            quote! {
                /// Iterate over the repeating field set entries.
                #[must_use]
                pub fn list(&self) -> #iter_name<'_> {
                    #offset_bits_expr
                    let remaining_bits = if self.raw.len() * 8 > offset_bits {
                        self.raw.len() * 8 - offset_bits
                    } else {
                        0
                    };
                    let count = if #set_bit_size > 0 { remaining_bits / #set_bit_size } else { 0 };
                    #iter_name {
                        data: &self.raw,
                        offset_bits,
                        remaining: count,
                        set_size_bits: #set_bit_size,
                    }
                }
            }
        };
        methods.push(iter_body);
    }

    if let Some(ref rs) = pgn.repeating_set2 {
        let iter_name = format_ident!("{}List2Iter", pgn.rust_name);

        let rep_fields: Vec<&ResolvedField> = pgn
            .fields
            .iter()
            .filter(|f| f.order >= rs.start_field && f.order < rs.start_field + rs.size)
            .collect();
        if rep_fields.is_empty() {
            return quote! { #(#methods)* };
        }
        let set_bit_size: usize = rep_fields.iter().map(|f| f.bit_length).sum();

        // Set2 starts after set1 — the caller must compute the offset dynamically.
        // We'll provide a method that takes the set1 count to compute the correct offset.
        let set1_start_bit = pgn.repeating_set1.as_ref().map_or(0, |rs1| {
            let rs1_fields: Vec<&ResolvedField> = pgn
                .fields
                .iter()
                .filter(|f| f.order >= rs1.start_field && f.order < rs1.start_field + rs1.size)
                .collect();
            rs1_fields.first().map_or(0, |f| f.bit_offset)
        });
        let set1_bit_size: usize = pgn.repeating_set1.as_ref().map_or(0, |rs1| {
            pgn.fields
                .iter()
                .filter(|f| f.order >= rs1.start_field && f.order < rs1.start_field + rs1.size)
                .map(|f| f.bit_length)
                .sum()
        });

        let count_field = rs
            .count_field
            .and_then(|cf_order| pgn.fields.iter().find(|f| f.order == cf_order));
        let count_expr = if let Some(cf) = count_field {
            let count_method = format_ident!("{}", cf.rust_name);
            quote! { self.#count_method().unwrap_or(0) as usize }
        } else {
            quote! { 0usize }
        };

        // Need set1 count to compute start offset for set2
        let set1_count_field = pgn.repeating_set1.as_ref().and_then(|rs1| {
            rs1.count_field
                .and_then(|cf_order| pgn.fields.iter().find(|f| f.order == cf_order))
        });
        let set1_count_expr = if let Some(cf) = set1_count_field {
            let count_method = format_ident!("{}", cf.rust_name);
            quote! { self.#count_method().unwrap_or(0) as usize }
        } else {
            quote! { self.list().count() }
        };

        methods.push(quote! {
            /// Iterate over the second repeating field set entries.
            #[must_use]
            pub fn list2(&self) -> #iter_name<'_> {
                let set1_count = #set1_count_expr;
                let offset_bits = #set1_start_bit + set1_count * #set1_bit_size;
                let count = #count_expr;
                #iter_name {
                    data: &self.raw,
                    offset_bits,
                    remaining: count,
                    set_size_bits: #set_bit_size,
                }
            }
        });
    }

    quote! { #(#methods)* }
}

/// Generate the Iterator and Entry structs for repeating field sets.
fn generate_repeating_sets(
    pgn: &ResolvedPgn,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let mut code = Vec::new();

    if let Some(ts) = pgn
        .repeating_set1
        .as_ref()
        .and_then(|rs| generate_one_repeating_set(pgn, rs, "List", enum_map))
    {
        code.push(ts);
    }
    if let Some(ts) = pgn
        .repeating_set2
        .as_ref()
        .and_then(|rs| generate_one_repeating_set(pgn, rs, "List2", enum_map))
    {
        code.push(ts);
    }

    quote! { #(#code)* }
}

fn generate_one_repeating_set(
    pgn: &ResolvedPgn,
    rs: &RepeatingSet,
    suffix: &str,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> Option<TokenStream> {
    let rep_fields: Vec<&ResolvedField> = pgn
        .fields
        .iter()
        .filter(|f| f.order >= rs.start_field && f.order < rs.start_field + rs.size)
        .collect();

    if rep_fields.is_empty() {
        return None;
    }

    let iter_name = format_ident!("{}{suffix}Iter", pgn.rust_name);
    let entry_name = format_ident!("{}{suffix}Entry", pgn.rust_name);

    // Generate entry accessors — each field's bit_offset is relative to the start of the set
    let first_bit_offset = rep_fields[0].bit_offset;
    let entry_accessors: Vec<TokenStream> = rep_fields
        .iter()
        .filter(|f| !matches!(f.kind, FieldKind::Reserved | FieldKind::Spare))
        .filter(|f| !f.is_variable_length)
        .map(|f| {
            let relative_offset = f.bit_offset.saturating_sub(first_bit_offset);
            generate_repeating_entry_accessor(f, relative_offset, enum_map)
        })
        .collect();

    Some(quote! {
        /// Iterator over repeating field set entries.
        pub struct #iter_name<'a> {
            data: &'a [u8],
            offset_bits: usize,
            remaining: usize,
            set_size_bits: usize,
        }

        impl<'a> Iterator for #iter_name<'a> {
            type Item = #entry_name<'a>;

            fn next(&mut self) -> Option<Self::Item> {
                if self.remaining == 0 {
                    return None;
                }
                let required_bytes = (self.offset_bits + self.set_size_bits + 7) / 8;
                if required_bytes > self.data.len() {
                    return None;
                }
                let entry = #entry_name {
                    data: self.data,
                    offset_bits: self.offset_bits,
                };
                self.offset_bits += self.set_size_bits;
                self.remaining -= 1;
                Some(entry)
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.remaining, Some(self.remaining))
            }
        }

        /// A single entry from a repeating field set.
        pub struct #entry_name<'a> {
            data: &'a [u8],
            offset_bits: usize,
        }

        impl #entry_name<'_> {
            #(#entry_accessors)*
        }
    })
}

/// Build doc attributes for a field accessor.
///
/// Returns `#[doc(alias = "Display Name")] #[doc = "description"]` when the
/// display name's `snake_case` differs from the Rust method name, otherwise
/// just `#[doc = "description"]`.
fn doc_attrs(field: &ResolvedField) -> TokenStream {
    let doc = field.description.as_deref().unwrap_or(&field.display_name);
    let alias_sn = field.display_name.to_snake_case();
    if alias_sn != field.rust_name && !matches!(field.kind, FieldKind::Reserved | FieldKind::Spare)
    {
        let alias = &field.display_name;
        quote! { #[doc(alias = #alias)] #[doc = #doc] }
    } else {
        quote! { #[doc = #doc] }
    }
}

/// Generate a field accessor for a repeating set entry.
/// The `relative_offset` is the bit offset within one repetition.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn generate_repeating_entry_accessor(
    field: &ResolvedField,
    relative_offset: usize,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let method_name = format_ident!("{}", field.rust_name);
    let bit_length = field.bit_length;
    let doc_attrs = doc_attrs(field);

    match &field.kind {
        FieldKind::Lookup {
            enum_name,
            rust_enum_name,
        } => {
            let enum_ident = format_ident!("{}", rust_enum_name);
            if enum_map.contains_key(enum_name) {
                let raw_method = format_ident!("{}_raw", field.rust_name);
                let raw_doc = format!("Raw value for `{}`.", field.display_name);
                quote! {
                    #[doc = #raw_doc]
                    #[must_use]
                    pub fn #raw_method(&self) -> Option<u64> {
                        let bit_offset = self.offset_bits + #relative_offset;
                        let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                    }

                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<super::lookups::#enum_ident> {
                        let bit_offset = self.offset_bits + #relative_offset;
                        let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { return None; }
                        super::lookups::#enum_ident::from_raw(raw as i64)
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        let bit_offset = self.offset_bits + #relative_offset;
                        let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                    }
                }
            }
        }
        FieldKind::Number if field.is_signed => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<i64> {
                            let bit_offset = self.offset_bits + #relative_offset;
                            let raw = nmea2000_types::field::extract_int(self.data, bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None } else { Some(raw) }
                        }
                    }
                } else {
                    let offset_add = if let Some(offset) = field.offset {
                        #[allow(clippy::cast_precision_loss)]
                        let o = offset as f64;
                        quote! { + #o }
                    } else {
                        quote! {}
                    };
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let bit_offset = self.offset_bits + #relative_offset;
                            let raw = nmea2000_types::field::extract_int(self.data, bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None }
                            else { Some(raw as f64 * #res #offset_add) }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<i64> {
                        let bit_offset = self.offset_bits + #relative_offset;
                        let raw = nmea2000_types::field::extract_int(self.data, bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None } else { Some(raw) }
                    }
                }
            }
        }
        FieldKind::Number => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<u64> {
                            let bit_offset = self.offset_bits + #relative_offset;
                            let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                        }
                    }
                } else {
                    let offset_add = if let Some(offset) = field.offset {
                        #[allow(clippy::cast_precision_loss)]
                        let o = offset as f64;
                        quote! { + #o }
                    } else {
                        quote! {}
                    };
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let bit_offset = self.offset_bits + #relative_offset;
                            let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None }
                            else { Some(raw as f64 * #res #offset_add) }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        let bit_offset = self.offset_bits + #relative_offset;
                        let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                    }
                }
            }
        }
        FieldKind::Mmsi | FieldKind::ManufacturerCode | FieldKind::PgnRef => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let bit_offset = self.offset_bits + #relative_offset;
                    let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                }
            }
        }
        FieldKind::Binary => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    let byte_start = (self.offset_bits + #relative_offset) / 8;
                    let end = (byte_start + #byte_len).min(self.data.len());
                    if byte_start >= self.data.len() { return &[]; }
                    &self.data[byte_start..end]
                }
            }
        }
        FieldKind::StringFix { .. } => {
            let byte_len = bit_length.div_ceil(8);
            let raw_method = format_ident!("{}_raw", field.rust_name);
            let raw_doc = format!("Raw bytes for `{}`.", field.display_name);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<&str> {
                    let byte_start = (self.offset_bits + #relative_offset) / 8;
                    if byte_start >= self.data.len() { return None; }
                    let end = (byte_start + #byte_len).min(self.data.len());
                    nmea2000_types::field::parse_string_fix(&self.data[byte_start..end])
                }

                #[doc = #raw_doc]
                #[must_use]
                pub fn #raw_method(&self) -> &[u8] {
                    let byte_start = (self.offset_bits + #relative_offset) / 8;
                    let end = (byte_start + #byte_len).min(self.data.len());
                    if byte_start >= self.data.len() { return &[]; }
                    &self.data[byte_start..end]
                }
            }
        }
        FieldKind::BitLookup { .. } => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let bit_offset = self.offset_bits + #relative_offset;
                    let raw = nmea2000_types::field::extract_uint(self.data, bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                }
            }
        }
        _ => quote! {},
    }
}

/// Generate a field accessor for a dynamic (Vec<u8>) PGN struct.
/// Like `generate_field_accessor` but adds a bounds check against `self.raw.len()`.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn generate_dynamic_field_accessor(
    field: &ResolvedField,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let method_name = format_ident!("{}", field.rust_name);
    let bit_offset = field.bit_offset;
    let bit_length = field.bit_length;
    let doc_attrs = doc_attrs(field);
    let required_bytes = (bit_offset + bit_length).div_ceil(8);

    match &field.kind {
        FieldKind::Lookup {
            enum_name,
            rust_enum_name,
        } => {
            let enum_ident = format_ident!("{}", rust_enum_name);
            if enum_map.contains_key(enum_name) {
                let raw_method = format_ident!("{}_raw", field.rust_name);
                let raw_doc = format!(
                    "Raw value for `{}`. Returns the numeric value before enum resolution.",
                    field.display_name
                );
                quote! {
                    #[doc = #raw_doc]
                    #[must_use]
                    pub fn #raw_method(&self) -> Option<u64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }

                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<super::lookups::#enum_ident> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            return None;
                        }
                        super::lookups::#enum_ident::from_raw(raw as i64)
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Number if field.is_signed => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<i64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw)
                            }
                        }
                    }
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res + #offset_f64)
                            }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res)
                            }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<i64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Number => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<u64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw)
                            }
                        }
                    }
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res + #offset_f64)
                            }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            if self.raw.len() < #required_bytes { return None; }
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res)
                            }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Latitude | FieldKind::Longitude => {
            let res = field.resolution.unwrap_or(1e-7);
            if field.is_signed {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<f64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                            None
                        } else {
                            Some(raw as f64 * #res)
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<f64> {
                        if self.raw.len() < #required_bytes { return None; }
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw as f64 * #res)
                        }
                    }
                }
            }
        }
        FieldKind::Float => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f32> {
                    if self.raw.len() < #required_bytes { return None; }
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, 32);
                    let val = f32::from_bits(raw as u32);
                    if val.is_nan() { None } else { Some(val) }
                }
            }
        }
        FieldKind::Date => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u16> {
                    if self.raw.len() < #required_bytes { return None; }
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw as u16)
                    }
                }
            }
        }
        FieldKind::Time => {
            let res = field.resolution.unwrap_or(0.0001);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f64> {
                    if self.raw.len() < #required_bytes { return None; }
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw as f64 * #res)
                    }
                }
            }
        }
        // Mmsi/ManufacturerCode/PgnRef: same as unsigned Number without resolution
        FieldKind::Mmsi | FieldKind::ManufacturerCode | FieldKind::PgnRef => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    if self.raw.len() < #required_bytes { return None; }
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                }
            }
        }
        FieldKind::Binary => {
            let byte_offset = bit_offset / 8;
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    if #byte_offset >= self.raw.len() { return &[]; }
                    &self.raw[#byte_offset..end]
                }
            }
        }
        FieldKind::StringFix { .. } => {
            let byte_offset = bit_offset / 8;
            let byte_len = bit_length.div_ceil(8);
            let raw_method = format_ident!("{}_raw", field.rust_name);
            let raw_doc = format!("Raw bytes for `{}`.", field.display_name);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<&str> {
                    if #byte_offset >= self.raw.len() { return None; }
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    nmea2000_types::field::parse_string_fix(&self.raw[#byte_offset..end])
                }

                #[doc = #raw_doc]
                #[must_use]
                pub fn #raw_method(&self) -> &[u8] {
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    if #byte_offset >= self.raw.len() { return &[]; }
                    &self.raw[#byte_offset..end]
                }
            }
        }
        FieldKind::StringLz | FieldKind::StringLau => {
            let byte_offset = bit_offset / 8;
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    if #byte_offset < self.raw.len() {
                        &self.raw[#byte_offset..]
                    } else {
                        &[]
                    }
                }
            }
        }
        FieldKind::BitLookup { .. } => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    if self.raw.len() < #required_bytes { return None; }
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw)
                    }
                }
            }
        }
        FieldKind::Decimal => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    if self.raw.len() < #required_bytes { return None; }
                    nmea2000_types::field::decode_decimal(&self.raw, #bit_offset, #bit_length)
                }
            }
        }
        FieldKind::Reserved | FieldKind::Spare => {
            quote! {}
        }
    }
}

/// Generate an accessor for a field whose offset must be computed at runtime.
///
/// This is used for any field that comes after a variable-length field (`STRING_LAU`/`LZ`).
/// The accessor starts at `last_fixed_byte_end` and scans past all `skip_items`
/// to find the actual byte offset at runtime.
#[allow(clippy::too_many_lines)]
fn generate_dynoffset_accessor(
    field: &ResolvedField,
    last_fixed_byte_end: usize,
    skip_items: &[DynOffsetItem],
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let method_name = format_ident!("{}", field.rust_name);
    let doc_attrs = doc_attrs(field);
    let bit_length = field.bit_length;

    // Generate skip steps (no early returns — callers check bounds afterward)
    let skip_steps = generate_skip_steps(skip_items);
    let has_skips = !skip_items.is_empty();

    let offset_setup = if has_skips {
        quote! { let mut offset = #last_fixed_byte_end; #(#skip_steps)* }
    } else {
        quote! { let offset = #last_fixed_byte_end; }
    };

    match &field.kind {
        FieldKind::StringLau => quote! {
            #doc_attrs
            #[must_use]
            pub fn #method_name(&self) -> Option<String> {
                let data = &self.raw;
                #offset_setup
                if offset >= data.len() { return None; }
                nmea2000_types::field::parse_string_lau(&data[offset..])
            }
        },
        FieldKind::StringLz => quote! {
            #doc_attrs
            #[must_use]
            pub fn #method_name(&self) -> Option<&str> {
                let data = &self.raw;
                #offset_setup
                if offset >= data.len() { return None; }
                nmea2000_types::field::parse_string_lz(&data[offset..])
            }
        },
        FieldKind::Number
        | FieldKind::Date
        | FieldKind::Time
        | FieldKind::Mmsi
        | FieldKind::ManufacturerCode
        | FieldKind::PgnRef => {
            let byte_len = bit_length.div_ceil(8);
            if field.is_signed {
                let res = field.resolution.unwrap_or(1.0);
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<i64> {
                            let data = &self.raw;
                            #offset_setup
                            if offset + #byte_len > data.len() { return None; }
                            let raw = nmea2000_types::field::extract_int(data, offset * 8, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None } else { Some(raw) }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let data = &self.raw;
                            #offset_setup
                            if offset + #byte_len > data.len() { return None; }
                            let raw = nmea2000_types::field::extract_int(data, offset * 8, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None } else { Some(raw as f64 * #res) }
                        }
                    }
                }
            } else {
                let res = field.resolution.unwrap_or(1.0);
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<u64> {
                            let data = &self.raw;
                            #offset_setup
                            if offset + #byte_len > data.len() { return None; }
                            let raw = nmea2000_types::field::extract_uint(data, offset * 8, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let data = &self.raw;
                            #offset_setup
                            if offset + #byte_len > data.len() { return None; }
                            let raw = nmea2000_types::field::extract_uint(data, offset * 8, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw as f64 * #res) }
                        }
                    }
                }
            }
        }
        FieldKind::Float => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f32> {
                    let data = &self.raw;
                    #offset_setup
                    if offset + #byte_len > data.len() { return None; }
                    let raw = nmea2000_types::field::extract_uint(data, offset * 8, 32);
                    let f = f32::from_bits(raw as u32);
                    if f.is_nan() { None } else { Some(f) }
                }
            }
        }
        FieldKind::Lookup {
            enum_name,
            rust_enum_name,
        } => {
            let enum_ident = format_ident!("{}", rust_enum_name);
            let byte_len = bit_length.div_ceil(8);
            if let Some(resolved_enum) = enum_map.get(enum_name.as_str()) {
                let match_arms: Vec<TokenStream> = resolved_enum
                    .variants
                    .iter()
                    .map(|v| {
                        let variant_ident = format_ident!("{}", v.rust_name);
                        let val = v.value;
                        quote! { #val => Some(super::lookups::#enum_ident::#variant_ident), }
                    })
                    .collect();
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<super::lookups::#enum_ident> {
                        let data = &self.raw;
                        #offset_setup
                        if offset + #byte_len > data.len() { return None; }
                        let raw = nmea2000_types::field::extract_uint(data, offset * 8, #bit_length);
                        match raw as i64 {
                            #(#match_arms)*
                            _ => None,
                        }
                    }
                }
            } else {
                // Fallback to raw value if enum not found
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        let data = &self.raw;
                        #offset_setup
                        if offset + #byte_len > data.len() { return None; }
                        let raw = nmea2000_types::field::extract_uint(data, offset * 8, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                    }
                }
            }
        }
        FieldKind::Latitude | FieldKind::Longitude => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f64> {
                    let data = &self.raw;
                    #offset_setup
                    if offset + #byte_len > data.len() { return None; }
                    let raw = nmea2000_types::field::extract_int(data, offset * 8, #bit_length);
                    if nmea2000_types::field::is_unknown_signed(raw, #bit_length) { None } else { Some(raw as f64 * 1e-16) }
                }
            }
        }
        FieldKind::Binary => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    let data = &self.raw;
                    #offset_setup
                    let end = (offset + #byte_len).min(data.len());
                    if offset >= data.len() { return &[]; }
                    &data[offset..end]
                }
            }
        }
        FieldKind::StringFix { .. } => {
            let byte_len = bit_length.div_ceil(8);
            let raw_method = format_ident!("{}_raw", field.rust_name);
            let raw_doc = format!("Raw bytes for `{}`.", field.display_name);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<&str> {
                    let data = &self.raw;
                    #offset_setup
                    if offset >= data.len() { return None; }
                    let end = (offset + #byte_len).min(data.len());
                    nmea2000_types::field::parse_string_fix(&data[offset..end])
                }

                #[doc = #raw_doc]
                #[must_use]
                pub fn #raw_method(&self) -> &[u8] {
                    let data = &self.raw;
                    #offset_setup
                    let end = (offset + #byte_len).min(data.len());
                    if offset >= data.len() { return &[]; }
                    &data[offset..end]
                }
            }
        }
        FieldKind::BitLookup { .. } => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let data = &self.raw;
                    #offset_setup
                    if offset + #byte_len > data.len() { return None; }
                    let raw = nmea2000_types::field::extract_uint(data, offset * 8, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) { None } else { Some(raw) }
                }
            }
        }
        FieldKind::Decimal => {
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let data = &self.raw;
                    #offset_setup
                    if offset + #byte_len > data.len() { return None; }
                    nmea2000_types::field::decode_decimal(data, offset * 8, #bit_length)
                }
            }
        }
        FieldKind::Reserved | FieldKind::Spare => quote! {},
    }
}

#[allow(clippy::too_many_lines)]
fn generate_field_accessor(
    field: &ResolvedField,
    enum_map: &HashMap<String, &ResolvedEnum>,
) -> TokenStream {
    let method_name = format_ident!("{}", field.rust_name);
    let bit_offset = field.bit_offset;
    let bit_length = field.bit_length;
    let doc_attrs = doc_attrs(field);

    match &field.kind {
        FieldKind::Lookup {
            enum_name,
            rust_enum_name,
        } => {
            let enum_ident = format_ident!("{}", rust_enum_name);
            // Check if enum actually exists
            if enum_map.contains_key(enum_name) {
                let raw_method = format_ident!("{}_raw", field.rust_name);
                let raw_doc = format!(
                    "Raw value for `{}`. Returns the numeric value before enum resolution.",
                    field.display_name
                );
                quote! {
                    #[doc = #raw_doc]
                    #[must_use]
                    pub fn #raw_method(&self) -> Option<u64> {
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }

                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<super::lookups::#enum_ident> {
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            return None;
                        }
                        super::lookups::#enum_ident::from_raw(raw as i64)
                    }
                }
            } else {
                // Fallback to raw value if enum not found
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Number if field.is_signed => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    // Integer with resolution 1, no offset
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<i64> {
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw)
                            }
                        }
                    }
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res + #offset_f64)
                            }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res)
                            }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<i64> {
                        let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Number => {
            // Unsigned number
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<u64> {
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw)
                            }
                        }
                    }
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res + #offset_f64)
                            }
                        }
                    }
                } else {
                    quote! {
                        #doc_attrs
                        #[must_use]
                        pub fn #method_name(&self) -> Option<f64> {
                            let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                            if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                                None
                            } else {
                                Some(raw as f64 * #res)
                            }
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<u64> {
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw)
                        }
                    }
                }
            }
        }
        FieldKind::Float => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f32> {
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, 32);
                    let val = f32::from_bits(raw as u32);
                    if val.is_nan() { None } else { Some(val) }
                }
            }
        }
        FieldKind::Latitude | FieldKind::Longitude => {
            let res = field.resolution.unwrap_or(1e-7);
            if field.is_signed {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<f64> {
                        let raw = nmea2000_types::field::extract_int(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_signed(raw, #bit_length) {
                            None
                        } else {
                            Some(raw as f64 * #res)
                        }
                    }
                }
            } else {
                quote! {
                    #doc_attrs
                    #[must_use]
                    pub fn #method_name(&self) -> Option<f64> {
                        let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                        if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                            None
                        } else {
                            Some(raw as f64 * #res)
                        }
                    }
                }
            }
        }
        FieldKind::Date => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u16> {
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw as u16)
                    }
                }
            }
        }
        FieldKind::Time => {
            let res = field.resolution.unwrap_or(0.0001);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<f64> {
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw as f64 * #res)
                    }
                }
            }
        }
        FieldKind::Mmsi | FieldKind::ManufacturerCode | FieldKind::PgnRef => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw)
                    }
                }
            }
        }
        FieldKind::Binary => {
            let byte_offset = bit_offset / 8;
            let byte_len = bit_length.div_ceil(8);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    &self.raw[#byte_offset..end]
                }
            }
        }
        FieldKind::StringFix { .. } => {
            let byte_offset = bit_offset / 8;
            let byte_len = bit_length.div_ceil(8);
            let raw_method = format_ident!("{}_raw", field.rust_name);
            let raw_doc = format!("Raw bytes for `{}`.", field.display_name);
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<&str> {
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    nmea2000_types::field::parse_string_fix(&self.raw[#byte_offset..end])
                }

                #[doc = #raw_doc]
                #[must_use]
                pub fn #raw_method(&self) -> &[u8] {
                    let end = (#byte_offset + #byte_len).min(self.raw.len());
                    &self.raw[#byte_offset..end]
                }
            }
        }
        FieldKind::StringLz | FieldKind::StringLau => {
            // Variable-length strings — return what we can
            let byte_offset = bit_offset / 8;
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> &[u8] {
                    if #byte_offset < self.raw.len() {
                        &self.raw[#byte_offset..]
                    } else {
                        &[]
                    }
                }
            }
        }
        FieldKind::BitLookup { .. } => {
            // Return raw bits; individual flag checking can use bitwise ops
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    let raw = nmea2000_types::field::extract_uint(&self.raw, #bit_offset, #bit_length);
                    if nmea2000_types::field::is_unknown_unsigned(raw, #bit_length) {
                        None
                    } else {
                        Some(raw)
                    }
                }
            }
        }
        FieldKind::Decimal => {
            quote! {
                #doc_attrs
                #[must_use]
                pub fn #method_name(&self) -> Option<u64> {
                    nmea2000_types::field::decode_decimal(&self.raw, #bit_offset, #bit_length)
                }
            }
        }
        FieldKind::Reserved | FieldKind::Spare => {
            quote! {}
        }
    }
}

#[allow(clippy::too_many_lines)]
fn generate_builder_setter(field: &ResolvedField) -> Option<TokenStream> {
    let method_name = format_ident!("{}", field.rust_name);
    let bit_offset = field.bit_offset;
    let bit_length = field.bit_length;
    let doc = format!("Set {}", field.display_name);

    match &field.kind {
        FieldKind::Number if field.is_signed => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: i64) -> Self {
                            nmea2000_types::field::insert_int(&mut self.raw, #bit_offset, #bit_length, value);
                            self
                        }
                    })
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: f64) -> Self {
                            let raw = ((value - #offset_f64) / #res) as i64;
                            nmea2000_types::field::insert_int(&mut self.raw, #bit_offset, #bit_length, raw);
                            self
                        }
                    })
                } else {
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: f64) -> Self {
                            let raw = (value / #res) as i64;
                            nmea2000_types::field::insert_int(&mut self.raw, #bit_offset, #bit_length, raw);
                            self
                        }
                    })
                }
            } else {
                Some(quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: i64) -> Self {
                        nmea2000_types::field::insert_int(&mut self.raw, #bit_offset, #bit_length, value);
                        self
                    }
                })
            }
        }
        FieldKind::Number => {
            if let Some(res) = field.resolution {
                if (res - 1.0).abs() < f64::EPSILON && field.offset.is_none() {
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: u64) -> Self {
                            nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, value);
                            self
                        }
                    })
                } else if let Some(offset) = field.offset {
                    #[allow(clippy::cast_precision_loss)]
                    let offset_f64 = offset as f64;
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: f64) -> Self {
                            let raw = ((value - #offset_f64) / #res) as u64;
                            nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, raw);
                            self
                        }
                    })
                } else {
                    Some(quote! {
                        #[doc = #doc]
                        pub fn #method_name(mut self, value: f64) -> Self {
                            let raw = (value / #res) as u64;
                            nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, raw);
                            self
                        }
                    })
                }
            } else {
                Some(quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: u64) -> Self {
                        nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, value);
                        self
                    }
                })
            }
        }
        FieldKind::Latitude | FieldKind::Longitude => {
            let res = field.resolution.unwrap_or(1e-7);
            if field.is_signed {
                Some(quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: f64) -> Self {
                        let raw = (value / #res) as i64;
                        nmea2000_types::field::insert_int(&mut self.raw, #bit_offset, #bit_length, raw);
                        self
                    }
                })
            } else {
                Some(quote! {
                    #[doc = #doc]
                    pub fn #method_name(mut self, value: f64) -> Self {
                        let raw = (value / #res) as u64;
                        nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, raw);
                        self
                    }
                })
            }
        }
        FieldKind::Lookup { .. }
        | FieldKind::BitLookup { .. }
        | FieldKind::Mmsi
        | FieldKind::ManufacturerCode
        | FieldKind::PgnRef => {
            let raw_method = format_ident!("{}_raw", field.rust_name);
            let raw_doc = format!("Set raw {} value", field.display_name);
            Some(quote! {
                #[doc = #raw_doc]
                pub fn #raw_method(mut self, value: u64) -> Self {
                    nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, value);
                    self
                }
            })
        }
        FieldKind::Date => Some(quote! {
            #[doc = #doc]
            pub fn #method_name(mut self, value: u16) -> Self {
                nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, value as u64);
                self
            }
        }),
        FieldKind::Time => {
            let res = field.resolution.unwrap_or(0.0001);
            Some(quote! {
                #[doc = #doc]
                pub fn #method_name(mut self, value: f64) -> Self {
                    let raw = (value / #res) as u64;
                    nmea2000_types::field::insert_uint(&mut self.raw, #bit_offset, #bit_length, raw);
                    self
                }
            })
        }
        FieldKind::StringFix { byte_length } => {
            let byte_offset = bit_offset / 8;
            let byte_len = *byte_length;
            Some(quote! {
                #[doc = #doc]
                pub fn #method_name(mut self, value: &str) -> Self {
                    let bytes = value.as_bytes();
                    let len = bytes.len().min(#byte_len);
                    self.raw[#byte_offset..#byte_offset + len].copy_from_slice(&bytes[..len]);
                    self
                }
            })
        }
        _ => None,
    }
}

/// Generate the PGN dispatch enum and registry.
fn generate_dispatch(pgns: &[ResolvedPgn]) -> TokenStream {
    use std::collections::BTreeMap;

    // Group PGNs by PGN number, preserving order
    let mut pgn_groups: BTreeMap<u32, Vec<&ResolvedPgn>> = BTreeMap::new();
    for p in pgns {
        pgn_groups.entry(p.pgn).or_default().push(p);
    }

    // Non-fallback PGNs become enum variants (deduplicated by struct name)
    let mut seen_names = std::collections::HashSet::new();
    let variants: Vec<TokenStream> = pgns
        .iter()
        .filter(|p| !p.is_fallback && seen_names.insert(&p.rust_name))
        .map(|p| {
            let variant_name = format_ident!("{}", p.rust_name);
            let module_name = format_ident!("{}", p.module_name);
            quote! {
                #variant_name(super::#module_name::#variant_name)
            }
        })
        .collect();

    // Generate decode arms — one per PGN number, with variant matching for multi-variant PGNs
    let decode_arms: Vec<TokenStream> = pgn_groups
        .iter()
        .map(|(&pgn_num, group)| generate_dispatch_arm(pgn_num, group))
        .collect();

    // Generate pgn() match arms — non-fallback variants only
    let mut seen_pgn_names = std::collections::HashSet::new();
    let pgn_arms: Vec<TokenStream> = pgns
        .iter()
        .filter(|p| !p.is_fallback && seen_pgn_names.insert(&p.rust_name))
        .map(|p| {
            let pgn_num = p.pgn;
            let variant_name = format_ident!("{}", p.rust_name);
            quote! {
                Self::#variant_name(_) => nmea2000_types::Pgn::new(#pgn_num)
            }
        })
        .collect();

    quote! {
        /// Decoded NMEA 2000 message, dispatched by PGN.
        #[derive(Debug, Clone)]
        pub enum DecodedMessage {
            #(#variants,)*
            /// PGN not in database.
            Unknown { pgn: nmea2000_types::Pgn, data: Vec<u8> },
        }

        impl DecodedMessage {
            /// Decode a raw message by PGN number.
            ///
            /// For PGNs with multiple variants (e.g. PGN 126208), the
            /// discriminating match field(s) are inspected to select the
            /// correct variant. Unrecognized variants fall back to a generic
            /// entry or `Unknown`.
            ///
            /// # Errors
            ///
            /// Returns an error if the payload data is too short for the given PGN.
            pub fn decode(pgn: u32, data: &[u8]) -> Result<Self, nmea2000_types::Error> {
                match pgn {
                    #(#decode_arms)*
                    _ => Ok(DecodedMessage::Unknown {
                        pgn: nmea2000_types::Pgn::new(pgn),
                        data: data.to_vec(),
                    }),
                }
            }

            /// Get the PGN of this message.
            #[must_use]
            pub fn pgn(&self) -> nmea2000_types::Pgn {
                match self {
                    #(#pgn_arms,)*
                    Self::Unknown { pgn, .. } => *pgn,
                }
            }
        }
    }
}

/// Generate a single dispatch match arm for a PGN number.
///
/// For single-variant PGNs: simple decode call.
/// For multi-variant PGNs: cascading match field checks, most specific first.
fn generate_dispatch_arm(pgn_num: u32, group: &[&ResolvedPgn]) -> TokenStream {
    // Only non-fallback variants get generated modules
    let specific: Vec<&&ResolvedPgn> = group.iter().filter(|p| !p.is_fallback).collect();

    // No specific variants (only fallback) — skip this PGN entirely
    if specific.is_empty() {
        return quote! {};
    }

    // Single variant, no match criteria — simple arm
    if specific.len() == 1 && specific[0].match_criteria.is_empty() {
        let p = specific[0];
        let variant_name = format_ident!("{}", p.rust_name);
        let module_name = format_ident!("{}", p.module_name);
        return quote! {
            #pgn_num => {
                super::#module_name::#variant_name::decode(data)
                    .map(DecodedMessage::#variant_name)
            }
        };
    }

    // Multi-variant: generate cascading match field checks
    // Sort by number of match criteria (most specific first)
    let mut sorted: Vec<&&ResolvedPgn> = specific.clone();
    sorted.sort_by(|a, b| b.match_criteria.len().cmp(&a.match_criteria.len()));

    // Check if all variants share a single match field at the same offset
    let single_field_dispatch = is_single_field_dispatch(&sorted);

    if let Some((bit_offset, bit_length)) = single_field_dispatch {
        // Optimized: single discriminant field → Rust match statement
        generate_single_field_dispatch(pgn_num, bit_offset, bit_length, &sorted)
    } else {
        // General case: compound match criteria → if-else chain
        generate_compound_dispatch(pgn_num, &sorted)
    }
}

/// Check if all variants use exactly one match field at the same bit position.
fn is_single_field_dispatch(variants: &[&&ResolvedPgn]) -> Option<(usize, usize)> {
    if variants.is_empty() {
        return None;
    }
    let first = &variants[0].match_criteria;
    if first.len() != 1 {
        return None;
    }
    let offset = first[0].bit_offset;
    let length = first[0].bit_length;
    if variants.iter().all(|v| {
        v.match_criteria.len() == 1
            && v.match_criteria[0].bit_offset == offset
            && v.match_criteria[0].bit_length == length
    }) {
        Some((offset, length))
    } else {
        None
    }
}

/// Generate a dispatch arm using a single `match` on one discriminant field.
fn generate_single_field_dispatch(
    pgn_num: u32,
    bit_offset: usize,
    bit_length: usize,
    variants: &[&&ResolvedPgn],
) -> TokenStream {
    let min_bytes = (bit_offset + bit_length).div_ceil(8);

    let match_arms: Vec<TokenStream> = variants
        .iter()
        .map(|p| {
            let value = p.match_criteria[0].expected_value as u64;
            let variant_name = format_ident!("{}", p.rust_name);
            let module_name = format_ident!("{}", p.module_name);
            quote! {
                #value => super::#module_name::#variant_name::decode(data)
                    .map(DecodedMessage::#variant_name),
            }
        })
        .collect();

    // Fallback PGNs don't get generated modules — always use Unknown as default
    let default_arm = quote! {
        _ => Ok(DecodedMessage::Unknown {
            pgn: nmea2000_types::Pgn::new(#pgn_num),
            data: data.to_vec(),
        }),
    };

    quote! {
        #pgn_num => {
            if data.len() >= #min_bytes {
                let disc = nmea2000_types::field::extract_uint(
                    data, #bit_offset, #bit_length,
                );
                match disc {
                    #(#match_arms)*
                    #default_arm
                }
            } else {
                Ok(DecodedMessage::Unknown {
                    pgn: nmea2000_types::Pgn::new(#pgn_num),
                    data: data.to_vec(),
                })
            }
        }
    }
}

/// Generate a dispatch arm using compound if-else checks for multi-field matching.
fn generate_compound_dispatch(pgn_num: u32, variants: &[&&ResolvedPgn]) -> TokenStream {
    // Separate variants with match criteria from those without
    let (with_criteria, without_criteria): (Vec<&&&ResolvedPgn>, Vec<&&&ResolvedPgn>) =
        variants.iter().partition(|p| !p.match_criteria.is_empty());

    let checks: Vec<TokenStream> = with_criteria
        .iter()
        .map(|p| {
            let variant_name = format_ident!("{}", p.rust_name);
            let module_name = format_ident!("{}", p.module_name);

            let min_bytes = p
                .match_criteria
                .iter()
                .map(|mc| (mc.bit_offset + mc.bit_length).div_ceil(8))
                .max()
                .unwrap_or(0);

            let conditions: Vec<TokenStream> = p
                .match_criteria
                .iter()
                .map(|mc| {
                    let offset = mc.bit_offset;
                    let length = mc.bit_length;
                    let value = mc.expected_value as u64;
                    quote! {
                        nmea2000_types::field::extract_uint(data, #offset, #length) == #value
                    }
                })
                .collect();

            quote! {
                if data.len() >= #min_bytes #(&& #conditions)* {
                    return super::#module_name::#variant_name::decode(data)
                        .map(DecodedMessage::#variant_name);
                }
            }
        })
        .collect();

    // Use the first variant without criteria as default, else Unknown
    let default = if let Some(p) = without_criteria.first() {
        let variant_name = format_ident!("{}", p.rust_name);
        let module_name = format_ident!("{}", p.module_name);
        quote! {
            super::#module_name::#variant_name::decode(data)
                .map(DecodedMessage::#variant_name)
        }
    } else {
        quote! {
            Ok(DecodedMessage::Unknown {
                pgn: nmea2000_types::Pgn::new(#pgn_num),
                data: data.to_vec(),
            })
        }
    };

    quote! {
        #pgn_num => {
            #(#checks)*
            #default
        }
    }
}

/// Main entry point: generate all Rust code from the canboat database.
///
/// # Errors
///
/// Returns an error if file I/O fails or the canboat database cannot be processed.
pub fn generate_all(
    db: &CanboatDatabase,
    out_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let generated_allow = "\
        #![allow(\n\
        \x20   clippy::unreadable_literal,\n\
        \x20   clippy::must_use_candidate,\n\
        \x20   clippy::return_self_not_must_use,\n\
        \x20   clippy::cast_possible_wrap,\n\
        \x20   clippy::cast_precision_loss,\n\
        \x20   clippy::missing_docs_in_private_items,\n\
        \x20   clippy::doc_markdown,\n\
        \x20   clippy::match_same_arms,\n\
        \x20   clippy::identity_op,\n\
        \x20   clippy::excessive_precision,\n\
        \x20   clippy::similar_names,\n\
        \x20   clippy::module_name_repetitions,\n\
        \x20   clippy::possible_missing_else,\n\
        \x20   clippy::len_zero,\n\
        \x20   clippy::absurd_extreme_comparisons,\n\
        \x20   clippy::manual_div_ceil,\n\
        \x20   clippy::needless_lifetimes,\n\
        \x20   unused_imports\n\
        )]\n";

    // Resolve enums
    let resolved_enums = resolve_enums(db);
    let enum_map: HashMap<String, &ResolvedEnum> =
        resolved_enums.iter().map(|e| (e.name.clone(), e)).collect();

    // Generate lookups module (enums + bit flags)
    let enum_code = generate_enum_code(&resolved_enums);
    let bit_flag_code = generate_bit_flag_code(db);
    let enum_file = format!(
        "{generated_allow}\
         #![allow(non_camel_case_types, clippy::upper_case_acronyms, clippy::enum_variant_names)]\n\
         {enum_code}\n{bit_flag_code}"
    );
    std::fs::write(out_dir.join("lookups.rs"), enum_file)?;

    // Resolve and generate PGN modules
    let mut resolved_pgns = Vec::new();
    let mut module_entries = Vec::new();

    for pgn_def in &db.pgns {
        let resolved = resolve_pgn(pgn_def);
        // Generate module files only for non-fallback PGNs
        if !resolved.is_fallback {
            let module_name = &resolved.module_name;
            let pgn_code = generate_pgn_code(&resolved, &enum_map);
            let pgn_file =
                format!("{generated_allow}use nmea2000_types::PgnMessage;\n\n{pgn_code}");
            std::fs::write(out_dir.join(format!("{module_name}.rs")), pgn_file)?;
            module_entries.push(module_name.clone());
        }
        resolved_pgns.push(resolved);
    }

    // Generate dispatch module
    let dispatch_code = generate_dispatch(&resolved_pgns);
    let dispatch_file = format!(
        "{generated_allow}\
         #![allow(unreachable_patterns)]\n\
         use nmea2000_types::PgnMessage;\n\n\
         {dispatch_code}"
    );
    std::fs::write(out_dir.join("dispatch.rs"), dispatch_file)?;

    // Generate top-level mod.rs
    let mod_entries: Vec<String> = module_entries
        .iter()
        .map(|m| format!("pub mod {m};"))
        .collect();
    let mod_file = format!(
        "pub mod lookups;\n{}\npub mod dispatch;\n\npub use dispatch::DecodedMessage;\n",
        mod_entries.join("\n")
    );
    std::fs::write(out_dir.join("mod.rs"), mod_file)?;

    Ok(())
}
