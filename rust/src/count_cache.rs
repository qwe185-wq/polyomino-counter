//! 已完成分项的跨进程数值缓存。这里不保存 DP 前沿，也不能恢复一行或一层。
//!
//! 校验和仅用于发现意外损坏；缓存目录及其文件必须由调用者信任，不能把校验和
//! 当作抵御恶意篡改的认证机制。

use num_bigint::BigUint;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAGIC: &str = "room-count-complete-count";
const SCHEMA: &str = "1";
// 数学对象、洞分类和 C4 旋转约定的版本；算法实现可变化，语义变化须递增。
const SEMANTIC_VERSION: &str = "fg4-bg4-bbox-exact-translation-c4-mirror-distinct-hole-v1";
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheKind {
    IdentityPlacement,
    Half,
    Quarter,
}

impl CacheKind {
    fn label(self) -> &'static str {
        match self {
            Self::IdentityPlacement => "identity-placement",
            Self::Half => "half",
            Self::Quarter => "quarter",
        }
    }

    fn rotation(self) -> &'static str {
        match self {
            Self::IdentityPlacement => "0",
            Self::Half => "180",
            Self::Quarter => "90",
        }
    }

    fn meaning(self) -> &'static str {
        match self {
            Self::IdentityPlacement => "placement-P",
            Self::Half | Self::Quarter => "fixed-exact-bbox",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedCounts {
    pub no_hole: BigUint,
    pub has_hole: BigUint,
}

pub struct CountCache {
    dir: PathBuf,
}

impl CountCache {
    pub fn new(path: &Path) -> io::Result<Self> {
        fs::create_dir_all(path)?;
        let dir = fs::canonicalize(path)?;
        if !dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "缓存路径不是目录",
            ));
        }
        Ok(Self { dir })
    }

    pub fn read(
        &self,
        kind: CacheKind,
        width: usize,
        height: usize,
    ) -> io::Result<Option<CachedCounts>> {
        validate_task(kind, width, height)?;
        let path = self.task_path(kind, width, height);
        let file = match File::open(path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(invalid_data("缓存文件过大"));
        }
        decode(&bytes, kind, width, height).map(Some)
    }

    pub fn write(
        &self,
        kind: CacheKind,
        width: usize,
        height: usize,
        counts: &CachedCounts,
    ) -> io::Result<()> {
        validate_task(kind, width, height)?;
        let target = self.task_path(kind, width, height);
        if target.exists() {
            return ensure_same(self.read(kind, width, height)?, counts);
        }
        let bytes = encode(kind, width, height, counts)?;
        let (mut file, mut temporary) = self.create_temp(&target)?;
        file.write_all(&bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);

        // hard_link 的目标创建是无覆盖操作。文件系统不支持时直接报错，
        // 不退回到可能覆盖并发写入者结果的 rename。
        match publish_no_replace(&temporary.path, &target) {
            Ok(()) => {
                fs::remove_file(&temporary.path)?;
                temporary.published = true;
                Ok(())
            }
            Err(err) => match self.read(kind, width, height)? {
                Some(existing) => ensure_same(Some(existing), counts),
                None => Err(err),
            },
        }
    }

    fn task_path(&self, kind: CacheKind, width: usize, height: usize) -> PathBuf {
        self.dir
            .join(format!("{}-{width}-{height}.count", kind.label()))
    }

    fn create_temp(&self, target: &Path) -> io::Result<(File, OwnedTemp)> {
        let stem = target
            .file_name()
            .expect("任务路径有文件名")
            .to_string_lossy();
        loop {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = self
                .dir
                .join(format!(".{stem}.{}.{}.tmp", std::process::id(), id));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    return Ok((
                        file,
                        OwnedTemp {
                            path,
                            published: false,
                        },
                    ))
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        }
    }
}

fn publish_no_replace(temporary: &Path, target: &Path) -> io::Result<()> {
    fs::hard_link(temporary, target)
}

struct OwnedTemp {
    path: PathBuf,
    published: bool,
}

impl Drop for OwnedTemp {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn validate_task(kind: CacheKind, width: usize, height: usize) -> io::Result<()> {
    if width == 0 || height == 0 || (kind == CacheKind::Quarter && width != height) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "无效缓存任务尺寸",
        ));
    }
    Ok(())
}

fn ensure_same(existing: Option<CachedCounts>, expected: &CachedCounts) -> io::Result<()> {
    if existing.as_ref() == Some(expected) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "已有缓存的计数不同，未覆盖",
        ))
    }
}

fn encode(
    kind: CacheKind,
    width: usize,
    height: usize,
    counts: &CachedCounts,
) -> io::Result<Vec<u8>> {
    let body = format!(
        "{MAGIC}\nschema={SCHEMA}\nsemantic={SEMANTIC_VERSION}\nkind={}\nwidth={width}\nheight={height}\nrotation={}\nmeaning={}\ncompletion=complete\nencoding=decimal-pair-v1\nno_hole={}\nhas_hole={}\n",
        kind.label(), kind.rotation(), kind.meaning(), counts.no_hole, counts.has_hole
    );
    let mut bytes = body.into_bytes();
    bytes.extend_from_slice(format!("checksum={:016x}\n", fnv1a64(&bytes)).as_bytes());
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "缓存计数过大"));
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], kind: CacheKind, width: usize, height: usize) -> io::Result<CachedCounts> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid_data("缓存不是 UTF-8"))?;
    if !text.ends_with('\n') {
        return Err(invalid_data("缓存被截断"));
    }
    let lines: Vec<&str> = text.split_terminator('\n').collect();
    if lines.len() != 13 {
        return Err(invalid_data("缓存字段数错误"));
    }
    let prefix_len = bytes.len() - lines[12].len() - 1;
    let checksum = field(lines[12], "checksum")?;
    if checksum.len() != 16
        || !checksum
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || checksum != format!("{:016x}", fnv1a64(&bytes[..prefix_len]))
    {
        return Err(invalid_data("缓存校验和错误"));
    }
    let expected = [
        MAGIC.to_owned(),
        format!("schema={SCHEMA}"),
        format!("semantic={SEMANTIC_VERSION}"),
        format!("kind={}", kind.label()),
        format!("width={width}"),
        format!("height={height}"),
        format!("rotation={}", kind.rotation()),
        format!("meaning={}", kind.meaning()),
        "completion=complete".to_owned(),
        "encoding=decimal-pair-v1".to_owned(),
    ];
    if lines[..10] != expected {
        return Err(invalid_data("缓存任务或协议不匹配"));
    }
    Ok(CachedCounts {
        no_hole: parse_count(field(lines[10], "no_hole")?)?,
        has_hole: parse_count(field(lines[11], "has_hole")?)?,
    })
}

fn field<'a>(line: &'a str, name: &str) -> io::Result<&'a str> {
    line.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('='))
        .ok_or_else(|| invalid_data("缓存字段名错误"))
}

fn parse_count(text: &str) -> io::Result<BigUint> {
    if text.is_empty()
        || (text.len() > 1 && text.starts_with('0'))
        || !text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid_data("缓存计数不是规范十进制"));
    }
    BigUint::parse_bytes(text.as_bytes(), 10).ok_or_else(|| invalid_data("缓存计数解析失败"))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf29ce484222325_u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let base = std::env::temp_dir();
            loop {
                let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
                let path = base.join(format!("room-count-cache-test-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(err) => panic!("无法建立测试目录: {err}"),
                }
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            // create_dir 成功后才持有该唯一路径，只清理自己创建的测试目录。
            fs::remove_dir_all(&self.0).expect("清理测试目录");
        }
    }

    fn counts(a: &str, b: &str) -> CachedCounts {
        CachedCounts {
            no_hole: BigUint::parse_bytes(a.as_bytes(), 10).unwrap(),
            has_hole: BigUint::parse_bytes(b.as_bytes(), 10).unwrap(),
        }
    }

    fn rewrite_body(path: &Path, body: &str) {
        let mut bytes = body.as_bytes().to_vec();
        bytes.extend_from_slice(format!("checksum={:016x}\n", fnv1a64(&bytes)).as_bytes());
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn roundtrip_huge_numbers_and_repeated_write() {
        let dir = TestDir::new();
        let cache = CountCache::new(&dir.0).unwrap();
        let huge = counts(&format!("1{}", "0".repeat(500)), "0");
        assert_eq!(
            cache.read(CacheKind::IdentityPlacement, 3, 40).unwrap(),
            None
        );
        cache
            .write(CacheKind::IdentityPlacement, 3, 40, &huge)
            .unwrap();
        cache
            .write(CacheKind::IdentityPlacement, 3, 40, &huge)
            .unwrap();
        assert_eq!(
            cache.read(CacheKind::IdentityPlacement, 3, 40).unwrap(),
            Some(huge)
        );
        let half = counts("123", "456");
        cache.write(CacheKind::Half, 3, 4, &half).unwrap();
        assert_eq!(cache.read(CacheKind::Half, 3, 4).unwrap(), Some(half));
        let quarter = counts("9", "11");
        cache.write(CacheKind::Quarter, 3, 3, &quarter).unwrap();
        assert_eq!(cache.read(CacheKind::Quarter, 3, 3).unwrap(), Some(quarter));
        assert_eq!(
            cache.read(CacheKind::Quarter, 3, 4).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rejects_corrupt_truncated_extra_and_wrong_task() {
        let dir = TestDir::new();
        let cache = CountCache::new(&dir.0).unwrap();
        let kind = CacheKind::Half;
        let path = cache.task_path(kind, 2, 3);
        let valid = encode(kind, 2, 3, &counts("12", "34")).unwrap();
        let mut corrupt = valid.clone();
        let pos = corrupt.iter().position(|&b| b == b'2').unwrap();
        corrupt[pos] = b'3';
        fs::write(&path, corrupt).unwrap();
        assert_eq!(
            cache.read(kind, 2, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(&path, &valid[..valid.len() - 5]).unwrap();
        assert_eq!(
            cache.read(kind, 2, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut extra = valid.clone();
        extra.extend_from_slice(b"extra\n");
        fs::write(&path, extra).unwrap();
        assert_eq!(
            cache.read(kind, 2, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(
            &path,
            encode(CacheKind::Quarter, 2, 2, &counts("12", "34")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            cache.read(kind, 2, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let bad_decimal = String::from_utf8(valid)
            .unwrap()
            .replace("no_hole=12\n", "no_hole=012\n");
        let body = bad_decimal.split("checksum=").next().unwrap();
        rewrite_body(&path, body);
        assert_eq!(
            cache.read(kind, 2, 3).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn existing_different_is_preserved_and_temporary_is_removed() {
        let dir = TestDir::new();
        let cache = CountCache::new(&dir.0).unwrap();
        let first = counts("1", "2");
        let second = counts("1", "3");
        cache.write(CacheKind::Half, 2, 3, &first).unwrap();
        let path = cache.task_path(CacheKind::Half, 2, 3);
        let original = fs::read(&path).unwrap();
        assert_eq!(
            cache
                .write(CacheKind::Half, 2, 3, &second)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn publication_cannot_replace_target_created_after_initial_check() {
        let dir = TestDir::new();
        let temporary = dir.0.join("candidate.tmp");
        let target = dir.0.join("result.count");
        fs::write(&temporary, b"new result").unwrap();
        assert!(!target.exists());
        // 模拟另一进程在初始检查之后、发布之前写入目标。
        fs::write(&target, b"other process result").unwrap();
        assert_eq!(
            publish_no_replace(&temporary, &target).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&target).unwrap(), b"other process result");
        assert_eq!(fs::read(&temporary).unwrap(), b"new result");
    }
}
