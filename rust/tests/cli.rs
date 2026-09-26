use std::process::Command;

#[test]
fn counting_defaults_to_transfer() {
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .arg("5")
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("前沿 DP"), "{text}");
    assert!(text.contains("520818"));
    assert!(text.contains("267976"));
    assert!(text.contains("252842"));
}

#[test]
fn invalid_dimensions_are_rejected() {
    // 旧版本会将0静默改成1；从不传入会触发6的值。
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .arg("0")
        .output()
        .unwrap();
    assert!(!output.status.success());
}

fn new_dir(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "room-count-cli-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

#[test]
fn raw_export_has_manifest_and_cannot_overwrite_it() {
    let dir = new_dir("raw");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["3", "--export", "--no-compress", "--export-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("One-sided BFS"));
    let manifest = std::fs::read(dir.join("dataset.json")).unwrap();
    assert!(String::from_utf8_lossy(&manifest).contains("\"count\": 46"));
    let data = std::fs::read(dir.join("all_fixed/shapes_000001.bin")).unwrap();
    assert_eq!(data.len(), 46 * 8);
    let second = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["1", "--export", "--no-compress", "--export-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert_eq!(std::fs::read(dir.join("dataset.json")).unwrap(), manifest);
    assert_eq!(
        std::fs::read(dir.join("all_fixed/shapes_000001.bin")).unwrap(),
        data
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compressor_failure_is_an_error_and_preserves_raw_data() {
    let dir = new_dir("failed-compressor");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["1", "--export", "--export-dir"])
        .arg(&dir)
        .env("ROOM_COUNT_7Z", dir.join("missing-compressor.exe"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.join("dataset.json").exists());
    assert_eq!(
        std::fs::read(dir.join("all_fixed/shapes_000001.bin")).unwrap(),
        1u64.to_le_bytes()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn transfer_does_not_silently_ignore_export() {
    let dir = new_dir("transfer-export");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["1", "--algorithm", "transfer", "--export", "--export-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.exists());
}
