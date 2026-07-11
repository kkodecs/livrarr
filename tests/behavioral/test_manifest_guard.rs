//! Guard: every `tests/behavioral/test_*.rs` file must be either registered
//! as a `[[test]]` target in `crates/livrarr-behavioral/Cargo.toml` or listed
//! in `PARKED` below with a reason. Prevents the manifest from silently
//! drifting out of sync with the files on disk again — see the 2026-07-11
//! orphan-test cleanup, `build/reports/orphan-test-triage-2026-07-11.md`
//! (30 files were found unregistered and had never been compiled).

use std::path::Path;

/// Files deliberately left unregistered pending the PO's standing
/// "commit the behavioral suite / CI" decision — see
/// `build/reports/orphan-test-triage-2026-07-11.md`. Anything else found in
/// `tests/behavioral/` must be registered in
/// `crates/livrarr-behavioral/Cargo.toml`.
const PARKED: &[&str] = &[
    "test_verify_e2.rs",
    "test_cup_convergence.rs",
    "test_metadata_redesign_phase3a.rs",
];

#[test]
fn all_behavioral_test_files_are_registered_or_parked() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/behavioral");
    let manifest = include_str!("../../crates/livrarr-behavioral/Cargo.toml");

    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
        .map(|entry| entry.expect("readable dir entry"))
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None; // skip subdirectories (fixtures/, ui/)
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("test_") && name.ends_with(".rs") {
                Some(name)
            } else {
                None // skips common.rs and anything else non-test
            }
        })
        .collect();
    on_disk.sort();

    let offending: Vec<String> = on_disk
        .into_iter()
        .filter(|name| {
            let path_form = format!("tests/behavioral/{name}");
            let registered = manifest.contains(path_form.as_str());
            let parked = PARKED.contains(&name.as_str());
            !registered && !parked
        })
        .collect();

    assert!(
        offending.is_empty(),
        "\n\nFound {} file(s) in tests/behavioral/ that are neither registered as a \
         [[test]] target in crates/livrarr-behavioral/Cargo.toml nor listed in this \
         guard's PARKED constant (tests/behavioral/test_manifest_guard.rs):\n\n  {}\n\n\
         To fix: either (a) add a [[test]] entry for it in \
         crates/livrarr-behavioral/Cargo.toml, or (b) add its filename to PARKED in \
         tests/behavioral/test_manifest_guard.rs with a comment explaining why it stays \
         unregistered.\n",
        offending.len(),
        offending.join("\n  "),
    );
}
