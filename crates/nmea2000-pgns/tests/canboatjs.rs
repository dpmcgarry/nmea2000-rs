//! Conformance tests against canboatjs's per-PGN decoder fixtures.
//!
//! Fixtures come from canboatjs `test/pgns/*.js`, converted to
//! `data/conformance/canboatjs/pgns.json` by `./scripts/update-canboatjs-tests.sh`.
//! Each case has a raw input (canboat plain, candump or YDRAW lines) and the
//! fields canboatjs decodes from it.
//!
//! canboatjs and this crate both report SI units, so values are compared
//! directly. Fields are matched by canboat field `Id`. Field metadata
//! (resolution, field type, bit lookups) comes from `data/canboat.json`.
//!
//! The decoded message is read through its generated `Debug` output, so no
//! test-only API is needed in the library. Repeating sets are not compared yet,
//! because `Debug` shows only their length.
//!
//! Mismatches that are already known live in
//! `data/conformance/canboatjs/known-failures.txt`. The test fails on any new
//! mismatch and on any listed check that now passes.
//!
//! - `CANBOATJS_BLESS=1` rewrites `known-failures.txt` from the current results.
//! - `CANBOATJS_REPORT=1` prints every check (use with `-- --nocapture`).
#![allow(clippy::cast_precision_loss)] // field values in fixtures fit in f64

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use nmea2000_pgns::DecodedMessage;
use nmea2000_types::{CanId, FastPacketReassembler, PacketType, Pgn};
use serde_json::Value;

const FIXTURES: &str = include_str!("../../../data/conformance/canboatjs/pgns.json");
const CANBOAT_DB: &str = include_str!("../../../data/canboat.json");
const KNOWN_FAILURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/conformance/canboatjs/known-failures.txt"
);

// ─── Input parsing ──────────────────────────────────────────────────────

struct Frame {
    prio: u8,
    pgn: u32,
    src: u8,
    dst: u8,
    data: Vec<u8>,
}

fn hex_bytes<'a>(it: impl Iterator<Item = &'a str>) -> Option<Vec<u8>> {
    it.map(|b| u8::from_str_radix(b.trim(), 16).ok()).collect()
}

fn from_can_id(id: &str, data: Vec<u8>) -> Option<Frame> {
    let id = CanId::from_raw(u32::from_str_radix(id, 16).ok()?)?;
    Some(Frame {
        prio: id.priority(),
        pgn: id.pgn().as_u32(),
        src: id.source_address(),
        dst: id.destination().unwrap_or(255),
        data,
    })
}

/// Parse one input line: canboat plain, candump, or YDRAW.
fn parse_line(line: &str) -> Option<Frame> {
    let parts: Vec<&str> = line.split(',').collect();
    if parts.len() >= 7 {
        // canboat plain: timestamp,prio,pgn,src,dst,len,b0,b1,...
        let len: usize = parts[5].trim().parse().ok()?;
        return Some(Frame {
            prio: parts[1].trim().parse().ok()?,
            pgn: parts[2].trim().parse().ok()?,
            src: parts[3].trim().parse().ok()?,
            dst: parts[4].trim().parse().ok()?,
            data: hex_bytes(parts[6..].iter().take(len).copied())?,
        });
    }
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() >= 3 && tokens[0].starts_with("can") && tokens[2].starts_with('[') {
        // candump: can0  0DF80523   [8]  C0 2B ...
        return from_can_id(tokens[1], hex_bytes(tokens[3..].iter().copied())?);
    }
    if tokens.len() >= 3 && (tokens[1] == "R" || tokens[1] == "T") {
        // YDRAW: 17:29:52.256 R 19F9050A 20 59 ...
        return from_can_id(tokens[2], hex_bytes(tokens[3..].iter().copied())?);
    }
    None
}

/// Turn the case input into (pgn, payload). Multi-line input is reassembled
/// as a fast packet.
fn payload(input: &Value) -> Result<(u32, Vec<u8>), String> {
    let lines: Vec<&str> = match input {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err("input is not a string or array".into()),
    };
    let frames: Vec<Frame> = lines
        .iter()
        .map(|l| parse_line(l).ok_or_else(|| format!("cannot parse input line: {l}")))
        .collect::<Result<_, _>>()?;
    if frames.len() == 1 {
        let f = frames.into_iter().next().unwrap();
        return Ok((f.pgn, f.data));
    }
    let pgn = frames[0].pgn;
    if Pgn::new(pgn).packet_type() != PacketType::FastPacket {
        return Err(format!(
            "{} frames for non fast-packet PGN {pgn}",
            frames.len()
        ));
    }
    let mut fp = FastPacketReassembler::new();
    for f in &frames {
        let id = CanId::new(f.prio, Pgn::new(f.pgn), f.dst, f.src);
        match fp.process_frame(id, &f.data) {
            Ok(Some(done)) => return Ok((pgn, done.data)),
            Ok(None) => {}
            Err(e) => return Err(format!("fast packet error: {e:?}")),
        }
    }
    Err("fast packet never completed".into())
}

// ─── Debug output parser ────────────────────────────────────────────────

/// A value read back from Rust `{:?}` output.
#[derive(Debug, Clone)]
#[allow(dead_code)] // struct names are parsed but only shown in failure output
enum Dv {
    Num(f64),
    Str(String),
    Ident(String),
    None,
    Struct(String, Vec<(String, Dv)>),
    Tuple(String, Vec<Dv>),
    List(Vec<Dv>),
}

struct DebugParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl DebugParser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn word(&mut self) -> String {
        self.ws();
        let start = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || b"_#.+-".contains(&self.s[self.i]))
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn string(&mut self) -> Option<String> {
        // Opening quote already checked by caller.
        self.i += 1;
        let mut bytes = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'"' => return Some(String::from_utf8_lossy(&bytes).into_owned()),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'n' => bytes.push(b'\n'),
                        b'r' => bytes.push(b'\r'),
                        b't' => bytes.push(b'\t'),
                        b'0' => bytes.push(0),
                        b'u' => {
                            // \u{XXXX}
                            let close = self.s[self.i..].iter().position(|&b| b == b'}')?;
                            let hex =
                                std::str::from_utf8(&self.s[self.i + 1..self.i + close]).ok()?;
                            let ch = char::from_u32(u32::from_str_radix(hex, 16).ok()?)?;
                            let mut buf = [0u8; 4];
                            bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            self.i += close + 1;
                        }
                        other => bytes.push(other),
                    }
                }
                other => bytes.push(other),
            }
        }
        None
    }

    fn seq(&mut self, close: u8) -> Option<Vec<Dv>> {
        let mut items = Vec::new();
        loop {
            if self.eat(close) {
                return Some(items);
            }
            items.push(self.value()?);
            if !self.eat(b',') {
                return self.eat(close).then_some(items);
            }
        }
    }

    fn value(&mut self) -> Option<Dv> {
        match self.peek()? {
            b'"' => self.string().map(Dv::Str),
            b'[' => {
                self.i += 1;
                self.seq(b']').map(Dv::List)
            }
            c if c.is_ascii_digit() || c == b'-' => self.word().parse().ok().map(Dv::Num),
            _ => {
                let name = self.word();
                if name.is_empty() {
                    return None;
                }
                if self.eat(b'{') {
                    let mut fields = Vec::new();
                    loop {
                        if self.eat(b'}') {
                            break;
                        }
                        let key = self.word();
                        if !self.eat(b':') {
                            return None;
                        }
                        fields.push((key, self.value()?));
                        if !self.eat(b',') {
                            if !self.eat(b'}') {
                                return None;
                            }
                            break;
                        }
                    }
                    Some(Dv::Struct(name, fields))
                } else if self.eat(b'(') {
                    self.seq(b')').map(|v| Dv::Tuple(name, v))
                } else {
                    match name.as_str() {
                        "None" => Some(Dv::None),
                        "inf" => Some(Dv::Num(f64::INFINITY)),
                        "NaN" => Some(Dv::Num(f64::NAN)),
                        _ => Some(Dv::Ident(name)),
                    }
                }
            }
        }
    }
}

fn parse_debug(s: &str) -> Option<Dv> {
    DebugParser {
        s: s.as_bytes(),
        i: 0,
    }
    .value()
}

fn unwrap_some(v: &Dv) -> &Dv {
    match v {
        Dv::Tuple(n, items) if n == "Some" && items.len() == 1 => unwrap_some(&items[0]),
        other => other,
    }
}

// ─── Comparison ─────────────────────────────────────────────────────────

/// Lowercase alphanumerics only, so `actualTemperature`, `actual_temperature`
/// and `r#type`/`type_` line up.
fn norm(s: &str) -> String {
    s.trim_start_matches("r#")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Days since 1970-01-01 for a canboat `YYYY.MM.DD` date.
fn days_from_date(s: &str) -> Option<i64> {
    let mut it = s.split(['.', '-']).map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (it.next()??, it.next()??, it.next()??);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

struct Db<'a> {
    pgns: &'a [Value],
    bit_lookups: BTreeMap<&'a str, BTreeMap<String, u32>>,
}

impl<'a> Db<'a> {
    fn new(root: &'a Value) -> Self {
        let mut bit_lookups = BTreeMap::new();
        for e in root["LookupBitEnumerations"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let bits = e["EnumBitValues"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    Some((
                        norm(b["Name"].as_str()?),
                        u32::try_from(b["Bit"].as_u64()?).ok()?,
                    ))
                })
                .collect();
            if let Some(name) = e["Name"].as_str() {
                bit_lookups.insert(name, bits);
            }
        }
        Db {
            pgns: root["PGNs"].as_array().unwrap(),
            bit_lookups,
        }
    }

    /// Pick the canboat.json entry for a case: by PGN and description, then
    /// by PGN and the Rust variant name, then the first entry for the PGN.
    fn entry(&self, pgn: u64, description: &str, variant: &str) -> Option<&'a Value> {
        let same_pgn: Vec<&Value> = self
            .pgns
            .iter()
            .filter(|p| p["PGN"].as_u64() == Some(pgn))
            .collect();
        same_pgn
            .iter()
            .find(|p| p["Description"].as_str() == Some(description))
            .or_else(|| {
                same_pgn
                    .iter()
                    .find(|p| norm(p["Id"].as_str().unwrap_or("")) == norm(variant))
            })
            .or_else(|| same_pgn.first())
            .copied()
    }
}

enum Outcome {
    Pass,
    Skip(String),
    Fail(String),
}

fn compare_field(db: &Db, def: Option<&Value>, expected: &Value, actual: Option<&Dv>) -> Outcome {
    let ft = def.and_then(|d| d["FieldType"].as_str()).unwrap_or("");
    if matches!(ft, "RESERVED" | "SPARE") {
        return Outcome::Skip("reserved/spare".into());
    }
    if let Value::Array(items) = expected {
        if ft != "BITLOOKUP" {
            return Outcome::Skip("repeating set not compared".into());
        }
        let lookup = def
            .and_then(|d| d["LookupBitEnumeration"].as_str())
            .unwrap_or("");
        let Some(bits) = db.bit_lookups.get(lookup) else {
            return Outcome::Fail(format!("unknown bit lookup {lookup}"));
        };
        let mut mask = 0u64;
        for name in items.iter().filter_map(Value::as_str) {
            match bits.get(&norm(name)) {
                Some(b) => mask |= 1 << b,
                None => return Outcome::Fail(format!("bit name {name:?} not in {lookup}")),
            }
        }
        return match actual.map(unwrap_some) {
            Some(Dv::Num(n)) if (*n - mask as f64).abs() < 0.5 => Outcome::Pass,
            other => Outcome::Fail(format!("expected bits {mask:#x}, got {other:?}")),
        };
    }

    let Some(actual) = actual else {
        return if expected.is_null() {
            Outcome::Skip("not exposed, expected null".into())
        } else {
            Outcome::Fail(format!("field not exposed, expected {expected}"))
        };
    };
    let mut actual = unwrap_some(actual).clone();

    // BINARY fields come back as raw bytes; canboatjs gives a little-endian
    // integer masked to the field width.
    if let (Dv::List(bytes), Value::Number(_)) = (&actual, expected) {
        let bits = def.and_then(|d| d["BitLength"].as_u64()).unwrap_or(64);
        if bits <= 64 {
            let mut v: u64 = 0;
            for (i, b) in bytes.iter().enumerate().take(8) {
                if let Dv::Num(n) = b {
                    v |= (*n as u64) << (8 * i);
                }
            }
            if bits < 64 {
                v &= (1u64 << bits) - 1;
            }
            actual = Dv::Num(v as f64);
        }
    }
    // canboatjs reports an empty string as null.
    if matches!(&actual, Dv::Str(s) if s.is_empty()) {
        actual = Dv::None;
    }
    // canboatjs scales some canboat units to SI; this crate keeps canboat units.
    let scale = match def.and_then(|d| d["Unit"].as_str()) {
        Some("%") => 0.01,
        Some("L") => 0.001,
        Some("L/h") => 1.0 / 3_600_000.0,
        Some("Ah") => 3600.0,
        Some("kWh") => 3_600_000.0,
        _ => 1.0,
    };
    if let Dv::Num(n) = &mut actual {
        *n *= scale;
    }
    let actual = &actual;

    match (expected, actual) {
        (Value::Null, Dv::None) => Outcome::Pass,
        (Value::Null, a) => Outcome::Fail(format!("expected null, got {a:?}")),
        (e, Dv::None) => Outcome::Fail(format!("expected {e}, got None")),
        (Value::Number(e), Dv::Num(a)) => {
            let e = e.as_f64().unwrap();
            let res = def.and_then(|d| d["Resolution"].as_f64()).unwrap_or(0.0) * scale;
            let tol = (res / 2.0).max(1e-9 * e.abs()).max(1e-12);
            if (a - e).abs() <= tol {
                Outcome::Pass
            } else {
                Outcome::Fail(format!("expected {e}, got {a} (tol {tol})"))
            }
        }
        (Value::String(e), Dv::Num(a)) => {
            let want = if ft == "DATE" {
                days_from_date(e).map(|d| d as f64)
            } else {
                e.trim().parse::<f64>().ok()
            };
            match want {
                Some(w) if (w - a).abs() < 0.5 => Outcome::Pass,
                Some(w) => Outcome::Fail(format!("expected {e:?} ({w}), got {a}")),
                None => Outcome::Fail(format!("expected {e:?}, got number {a}")),
            }
        }
        // Variants for names that start with a digit get a `V` prefix ("12V" -> V12v).
        (Value::String(e), Dv::Ident(a))
            if norm(e) == norm(a) || format!("v{}", norm(e)) == norm(a) =>
        {
            Outcome::Pass
        }
        (Value::String(e), Dv::Str(a)) if e.trim() == a.trim() => Outcome::Pass,
        (e, a) => Outcome::Fail(format!("expected {e}, got {a:?}")),
    }
}

/// Run one case; returns (check name, outcome) pairs. Check name `<case>`
/// covers input and decode problems.
fn run_case(db: &Db, case: &Value) -> Vec<(String, Outcome)> {
    let exp = &case["expected"];
    let pgn = exp["pgn"].as_u64().unwrap_or(0);
    let description = exp["description"].as_str().unwrap_or("");

    let (in_pgn, data) = match payload(&case["input"]) {
        Ok(p) => p,
        Err(e) => return vec![("<case>".into(), Outcome::Fail(e))],
    };
    if u64::from(in_pgn) != pgn {
        return vec![(
            "<case>".into(),
            Outcome::Fail(format!("input PGN {in_pgn} != {pgn}")),
        )];
    }
    let decoded = match DecodedMessage::decode(in_pgn, &data) {
        Ok(d) => d,
        Err(e) => {
            return vec![(
                "<case>".into(),
                Outcome::Fail(format!("decode error: {e:?}")),
            )];
        }
    };
    if matches!(decoded, DecodedMessage::Unknown { .. }) {
        return vec![("<case>".into(), Outcome::Fail("decoded as Unknown".into()))];
    }
    let dbg = format!("{decoded:?}");
    let (variant, fields) = match parse_debug(&dbg) {
        Some(Dv::Tuple(v, items)) => match items.into_iter().next() {
            Some(Dv::Struct(_, f)) => (v, f),
            _ => {
                return vec![(
                    "<case>".into(),
                    Outcome::Fail(format!("unexpected Debug shape: {dbg}")),
                )];
            }
        },
        _ => {
            return vec![(
                "<case>".into(),
                Outcome::Fail(format!("cannot parse Debug: {dbg}")),
            )];
        }
    };

    let entry = db.entry(pgn, description, &variant);
    let mut out = Vec::new();
    if let Some(e) = entry {
        let id = e["Id"].as_str().unwrap_or("");
        if e["Description"].as_str() == Some(description) && norm(id) != norm(&variant) {
            out.push((
                "<case>".into(),
                Outcome::Fail(format!(
                    "decoded as {variant}, expected {id} ({description})"
                )),
            ));
            return out;
        }
    }
    let defs: Vec<&Value> = entry
        .and_then(|e| e["Fields"].as_array())
        .map_or_else(Vec::new, |f| f.iter().collect());

    for (key, expected) in exp["fields"].as_object().into_iter().flatten() {
        let def = defs.iter().find(|d| d["Id"].as_str() == Some(key)).copied();
        let actual = fields
            .iter()
            .find(|(k, _)| norm(k) == norm(key))
            .map(|(_, v)| v);
        out.push((key.clone(), compare_field(db, def, expected, actual)));
    }
    out
}

// ─── Test ───────────────────────────────────────────────────────────────

#[test]
fn canboatjs_fixtures() {
    let root: Value = serde_json::from_str(CANBOAT_DB).expect("canboat.json");
    let db = Db::new(&root);
    let cases: Vec<Value> = serde_json::from_str(FIXTURES).expect("pgns.json");
    assert!(!cases.is_empty(), "no canboatjs fixtures");

    let report = std::env::var_os("CANBOATJS_REPORT").is_some();
    let (mut pass, mut skip) = (0usize, 0usize);
    let mut failures: BTreeMap<String, String> = BTreeMap::new();
    let mut per_pgn: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();

    for case in &cases {
        if case["disabled"].as_bool() == Some(true) {
            continue;
        }
        let file = case["file"].as_str().unwrap_or("?");
        let idx = case["index"].as_u64().unwrap_or(0);
        let stats = per_pgn
            .entry(file.trim_end_matches(".js").to_string())
            .or_default();
        for (check, outcome) in run_case(&db, case) {
            let key = format!("{file}#{idx}/{check}");
            match outcome {
                Outcome::Pass => {
                    pass += 1;
                    stats.0 += 1;
                    if report {
                        println!("PASS {key}");
                    }
                }
                Outcome::Skip(why) => {
                    skip += 1;
                    stats.2 += 1;
                    if report {
                        println!("SKIP {key}  {why}");
                    }
                }
                Outcome::Fail(why) => {
                    stats.1 += 1;
                    if report {
                        println!("FAIL {key}  {why}");
                    }
                    failures.insert(key, why);
                }
            }
        }
    }

    if report {
        println!("\n{:>8} {:>5} {:>5} {:>5}", "PGN", "pass", "fail", "skip");
        for (pgn, (p, f, s)) in &per_pgn {
            println!("{pgn:>8} {p:>5} {f:>5} {s:>5}");
        }
    }
    println!(
        "canboatjs fixtures: {} cases, {pass} pass, {} fail, {skip} skip",
        cases.len(),
        failures.len()
    );

    if std::env::var_os("CANBOATJS_BLESS").is_some() {
        let mut text = String::from(
            "# Known mismatches against canboatjs fixtures.\n\
             # Regenerate: CANBOATJS_BLESS=1 cargo test -p nmea2000-pgns --test canboatjs\n",
        );
        for (k, why) in &failures {
            let why = why.replace('\n', " ");
            let _ = writeln!(text, "{k}  # {why}");
        }
        std::fs::write(KNOWN_FAILURES, text).expect("write known-failures.txt");
        return;
    }

    let known: BTreeSet<String> = std::fs::read_to_string(KNOWN_FAILURES)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();

    let new: Vec<String> = failures
        .iter()
        .filter(|(k, _)| !known.contains(*k))
        .map(|(k, why)| format!("  {k}  {why}"))
        .collect();
    let fixed: Vec<&String> = known
        .iter()
        .filter(|k| !failures.contains_key(*k))
        .collect();

    let mut msg = String::new();
    if !new.is_empty() {
        let _ = writeln!(msg, "{} new mismatch(es):\n{}", new.len(), new.join("\n"));
    }
    if !fixed.is_empty() {
        let _ = writeln!(
            msg,
            "{} known failure(s) now pass; remove them from known-failures.txt:\n  {}",
            fixed.len(),
            fixed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    assert!(msg.is_empty(), "{msg}");
}
