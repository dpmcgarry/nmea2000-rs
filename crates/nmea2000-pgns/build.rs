use std::path::PathBuf;

fn main() {
    let canboat_json = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("data/canboat.json");

    println!("cargo:rerun-if-changed={}", canboat_json.display());

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let db =
        nmea2000_codegen::parse_canboat_json(&canboat_json).expect("Failed to parse canboat.json");
    nmea2000_codegen::generate_all(&db, &out_dir).expect("Failed to generate PGN code");
}
