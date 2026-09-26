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
fn dynamic_cache_parallel_and_profile_preserve_categories() {
    let dir = new_dir("count-cache");
    let run = |n: &str, threads: &str| {
        Command::new(env!("CARGO_BIN_EXE_room-count"))
            .args([
                n,
                "--algorithm",
                "transfer",
                "--profile-count",
                "--count-threads",
                threads,
                "--count-cache-dir",
            ])
            .arg(&dir)
            .output()
            .unwrap()
    };
    let first = run("4", "1");
    let expanded = run("5", "3");
    let warm = run("5", "3");
    for output in [&first, &expanded, &warm] {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let rows = |bytes: &[u8]| {
        String::from_utf8_lossy(bytes)
            .lines()
            .filter(|line| line.starts_with("n=") && line.contains("total="))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&expanded.stdout), rows(&warm.stdout));
    assert_eq!(
        rows(&expanded.stdout).last().unwrap(),
        "n=5: total=520818, no_hole=267976, has_hole=252842"
    );
    let warm_log = String::from_utf8_lossy(&warm.stderr);
    assert!(warm_log.contains("count_cache hit=identity"));
    assert!(warm_log.contains("count_cache hit=symmetry"));
    assert!(!warm_log.contains("count_stage phase=begin"));
    // 只清理本测试创建的独立目录。
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_count_options_fail_before_enumeration() {
    for arguments in [
        vec!["1", "--count-threads", "0"],
        vec!["1", "--count-memory-mib", "0"],
        vec!["1", "--export", "--count-threads", "2"],
        vec!["1", "--export", "--count-threads", "1"],
        vec!["1", "--algorithm", "bfs", "--count-memory-mib", "4096"],
        vec!["1", "--algorithm", "bfs", "--profile-count"],
    ] {
        assert!(!Command::new(env!("CARGO_BIN_EXE_room-count"))
            .args(arguments)
            .output()
            .unwrap()
            .status
            .success());
    }
}

#[test]
fn explicit_symmetry_engines_agree_on_known_counts() {
    for engine in ["frontier", "quotient", "gray"] {
        let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
            .args(["5", "--symmetry-engine", engine])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{engine}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout)
            .contains("n=5: total=520818, no_hole=267976, has_hole=252842"));
    }
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("前沿状态图回溯"));
    let manifest = std::fs::read(dir.join("dataset.json")).unwrap();
    assert!(String::from_utf8_lossy(&manifest).contains("\"count\": 46"));
    let data = std::fs::read(dir.join("no_holes/n01_fixed/shapes_000001.bin")).unwrap();
    assert_eq!(data, 1u64.to_le_bytes());
    let second = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["1", "--export", "--no-compress", "--export-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert_eq!(std::fs::read(dir.join("dataset.json")).unwrap(), manifest);
    assert_eq!(
        std::fs::read(dir.join("no_holes/n01_fixed/shapes_000001.bin")).unwrap(),
        data
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compressor_failure_is_an_error_and_preserves_raw_data() {
    let dir = new_dir("failed-compressor");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args([
            "1",
            "--export",
            "--compression-backend",
            "7z",
            "--export-dir",
        ])
        .arg(&dir)
        .env("ROOM_COUNT_7Z", dir.join("missing-compressor.exe"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.join("dataset.json").exists());
    assert_eq!(
        std::fs::read(dir.join("no_holes/n01_fixed/shapes_000001.bin")).unwrap(),
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

#[test]
fn native_zip_does_not_require_external_compressor() {
    use std::io::Read;
    let dir = new_dir("native");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["3", "--export", "--export-dir"])
        .arg(&dir)
        .env("ROOM_COUNT_7Z", dir.join("missing-compressor.exe"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut count = 0;
    for category in ["no_holes", "with_holes"] {
        for n in 1..=3 {
            let path = dir.join(category).join(format!("n{n:02}_fixed.zip"));
            if !path.exists() {
                continue;
            }
            let mut archive = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
            for i in 0..archive.len() {
                let mut bytes = Vec::new();
                archive
                    .by_index(i)
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                assert_eq!(bytes.len() % 8, 0);
                count += bytes.len() / 8;
            }
            assert!(!path.with_extension("zip.partial").exists());
        }
    }
    assert_eq!(count, 46);
    assert!(dir.join("dataset.json").exists());
    assert!(!dir.join("all_fixed.zip").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn export_stores_each_shape_once_in_classified_streams() {
    let dir = new_dir("single-copy");
    let output = Command::new(env!("CARGO_BIN_EXE_room-count"))
        .args(["3", "--export", "--no-compress", "--export-dir"])
        .arg(&dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let manifest = std::fs::read_to_string(dir.join("dataset.json")).unwrap();
    let has_all_copy = dir.join("all_fixed").exists();
    let mut bytes = 0;
    for category in ["no_holes", "with_holes"] {
        for n in 1..=3 {
            let file = dir
                .join(category)
                .join(format!("n{n:02}_fixed/shapes_000001.bin"));
            if file.exists() {
                bytes += std::fs::metadata(file).unwrap().len();
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert!(!has_all_copy, "每个形状只能写所属分类，不再复制all");
    assert_eq!(bytes, 46 * 8);
    assert!(manifest.contains("\"format_version\": 2"));
    assert!(manifest.contains("classified-single-copy"));
    assert!(manifest.contains("\"streams\""));
}
