//! Builds every translation in `po/` into the binary: writes `catalogs.rs`
//! with one `(language, contents)` pair per `.po` file.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let po = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("po");
    println!("cargo:rerun-if-changed={}", po.display());
    let mut entries: Vec<_> = fs::read_dir(&po)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "po"))
        .collect();
    entries.sort();
    let mut out = String::from("pub const CATALOGS: &[(&str, &str)] = &[\n");
    for path in entries {
        println!("cargo:rerun-if-changed={}", path.display());
        let lang = path.file_stem().unwrap().to_str().unwrap();
        out.push_str(&format!(
            "    ({lang:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    out.push_str("];\n");
    let dest = PathBuf::from(env::var("OUT_DIR").unwrap()).join("catalogs.rs");
    fs::write(dest, out).unwrap();
}
