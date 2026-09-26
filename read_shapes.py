"""
从 Fixed mask 二进制文件中提取和可视化多联骨牌形状。

二进制格式: v1/v2 为 8 字节 u64 little-endian；v3 为清单指定宽度的无符号小端整数
位图编码: 行优先, stride 由格式确定, 左上角对齐 (0,0)

输出按 10M 个 mask 切分为独立 chunk，zip 内每个 chunk 是独立条目，
读取时只解压目标 chunk，不解压整个文件。

用法:
  python read_shapes.py <数据集目录|文件.(bin|zip)|chunk目录> [选项]

选项:
  --ascii      输出 ASCII 图形
  --txt        输出紧凑文本 (默认)
  --info       仅显示文件摘要 (zip: 零解压)
  --from N     从第 N 个开始 (1-based)
  --to M       到第 M 个为止 (含)
  --limit N    只显示前 N 个
  --hole       仅显示有洞的形状
  --nohole     仅显示无洞的形状

示例:
  python read_shapes.py data.zip --info
  python read_shapes.py data.zip --txt --limit 10
  python read_shapes.py data.zip --ascii --from 100000000 --to 100000005
"""

import sys
import struct
import os
import re
import zipfile
import json

STRIDE = 8
MASK_SIZE = 8
CHUNK_SIZE = 10_000_000  # 与 Rust export.rs 一致
READ_BATCH_SIZE = 256 * 1024


# ================================================================
# Mask 操作
# ================================================================

def mask_to_cells(mask, stride=STRIDE):
    cells = []
    while mask:
        lsb = mask & -mask
        bit = (lsb.bit_length() - 1)
        r, c = divmod(bit, stride)
        cells.append((c, r))
        mask ^= lsb
    return cells


def mask_extent(mask, stride=STRIDE):
    cells = mask_to_cells(mask, stride)
    if not cells:
        return (0, 0)
    return (max(c for c, r in cells) + 1, max(r for c, r in cells) + 1)


def mask_popcount(mask):
    return mask.bit_count()


def shape_to_ascii(mask, stride=STRIDE):
    cells = set(mask_to_cells(mask, stride))
    w, h = mask_extent(mask, stride)
    grid = [["  " for _ in range(w)] for _ in range(h)]
    for c, r in cells:
        grid[h - 1 - r][c] = "██"
    return "\n".join("".join(row) for row in grid)


def shape_to_text(mask, stride=STRIDE):
    cells = mask_to_cells(mask, stride)
    cells_str = " ".join(f"({c},{r})" for c, r in cells)
    size = mask_popcount(mask)
    w, h = mask_extent(mask, stride)
    return f"[size={size} {w}x{h}] {cells_str}"


# ================================================================
# Chunk 索引
# ================================================================

CHUNK_RE = re.compile(r"shapes_([0-9]{6,})\.bin\Z")
STREAM_RE = re.compile(r"(no_holes|with_holes)/n([0-9]{2,})_fixed(\.zip)?$")
DECIMAL_RE = re.compile(r"(?:0|[1-9][0-9]*)\Z")


def _decimal_count(value, name, positive=False):
    if not isinstance(value, str) or not DECIMAL_RE.fullmatch(value):
        raise ValueError(f"Invalid {name}")
    count = int(value)
    if positive and count == 0:
        raise ValueError(f"Invalid {name}")
    return count


def _v3_properties(manifest):
    if manifest.get("encoding") != "uint-le-fixed":
        raise ValueError("Unsupported mask encoding")
    for name in ("max_n", "stride", "record_bytes"):
        if type(manifest.get(name)) is not int or manifest[name] <= 0:
            raise ValueError(f"Invalid {name}")
    max_n = manifest["max_n"]
    stride = max(8, max_n)
    record_bytes = max(8, (max_n * stride + 7) // 8)
    if manifest["stride"] != stride or manifest["record_bytes"] != record_bytes:
        raise ValueError("Invalid v3 stride or record_bytes")
    if manifest.get("category_dimension") != "exact-max-bounding-box":
        raise ValueError("Unsupported category dimension")
    if manifest.get("equivalence") != "one-sided":
        raise ValueError("Unsupported equivalence")
    return stride, record_bytes, max_n * stride


def _checked_count(size, name, chunk=True, record_bytes=MASK_SIZE):
    if size % record_bytes:
        raise ValueError(f"Mask data length is not divisible by {record_bytes}: {name}")
    count = size // record_bytes
    if chunk and count > CHUNK_SIZE:
        raise ValueError(f"Chunk exceeds {CHUNK_SIZE} masks: {name}")
    return count


def _read_masks(file, count, name, record_bytes=MASK_SIZE, used_bits=64):
    """以固定大小批量解码，短读必须报错。"""
    batch_count = max(1, READ_BATCH_SIZE // record_bytes)
    while count:
        take = min(count, batch_count)
        expected = take * record_bytes
        data = file.read(expected)
        if len(data) != expected:
            raise ValueError(f"Short mask read: {name}")
        if record_bytes == MASK_SIZE and used_bits == 64:
            # 旧8字节数据保留批量unpack快路径。
            for (mask,) in struct.iter_unpack("<Q", data):
                yield mask
        else:
            for offset in range(0, expected, record_bytes):
                mask = int.from_bytes(data[offset:offset + record_bytes], "little")
                if mask >> used_bits:
                    raise ValueError(f"Unused high bits set in mask: {name}")
                yield mask
        count -= take


class ChunkIndex:
    """zip 内多个 chunk 文件的索引，支持按全局 mask 编号定位到具体 chunk 和偏移。"""

    def __init__(self, zip_path: str, stride=STRIDE, record_bytes=MASK_SIZE,
                 used_bits=64, strict=False):
        self.chunks = []  # [(start_global_idx, entry_name, file_size)]
        self.total = 0
        self.path = zip_path
        self.stride = stride
        self.record_bytes = record_bytes
        self.used_bits = used_bits
        self.strict = strict
        self._build(zip_path)

    def _build(self, zip_path: str):
        with zipfile.ZipFile(zip_path, "r") as zf:
            entries = []
            for info in zf.infolist():
                name = info.filename
                m = CHUNK_RE.fullmatch(name.rsplit("/", 1)[-1])
                if m:
                    entries.append((int(m.group(1)), name, info.file_size))
                elif self.strict and name.endswith(".bin"):
                    raise ValueError(f"Invalid v3 ZIP chunk name: {name}")

            if not entries and not self.strict:
                # 兼容旧格式：只有一个 shapes_0001.bin 或无编号
                for info in zf.infolist():
                    name = info.filename
                    if name.endswith(".bin"):
                        entries.append((1, name, info.file_size))
                        break

            if not entries:
                raise FileNotFoundError(f"No .bin found in {zip_path}")

            entries.sort(key=lambda x: x[0])
            seen_names = set()
            seen_numbers = set()
            for number, name, size in entries:
                if name in seen_names:
                    raise ValueError(f"Duplicate ZIP entry: {name}")
                if self.strict and number in seen_numbers:
                    raise ValueError(f"Duplicate v3 ZIP chunk number: {number}")
                seen_names.add(name)
                seen_numbers.add(number)
                count = _checked_count(size, name,
                                       CHUNK_RE.fullmatch(name.rsplit("/", 1)[-1]) is not None,
                                       self.record_bytes)
                self.chunks.append((self.total, name, count))
                self.total += count

    def iter_range(self, start: int, end: int):
        """生成 mask 迭代器，start/end 为全局 0-based 索引（含）。"""
        for global_start, name, count in self.chunks:
            global_end = global_start + count - 1
            if global_end < start:
                continue
            if global_start > end:
                break

            # 计算该 chunk 内的偏移和读取长度
            skip = max(0, start - global_start)
            read_count = min(count - skip, end - max(start, global_start) + 1)
            byte_offset = skip * self.record_bytes
            # 按需打开 ZIP；清单扫描及 --info 不保留文件句柄。
            with zipfile.ZipFile(self.path, "r") as zf:
                with zf.open(name, "r") as f:
                    f.seek(byte_offset)
                    yield from _read_masks(f, read_count, name, self.record_bytes,
                                           self.used_bits)

    def close(self):
        # iter_range 中的 with 块在迭代结束或关闭时释放句柄。
        pass


# ================================================================
# .bin 单文件读取（也支持 chunk 化目录）
# ================================================================

class BinIndex:
    """单 .bin 文件的索引（chunk 化目录或裸文件）。"""

    def __init__(self, path: str, stride=STRIDE, record_bytes=MASK_SIZE,
                 used_bits=64, strict=False):
        self.path = path
        self.stride = stride
        self.record_bytes = record_bytes
        self.used_bits = used_bits
        self.strict = strict
        if os.path.isdir(path):
            self.chunks = []
            self.total = 0
            entries = []
            for name in os.listdir(path):
                m = CHUNK_RE.fullmatch(name)
                if m:
                    entries.append((int(m.group(1)), name))
                elif strict and name.endswith(".bin"):
                    raise ValueError(f"Invalid v3 chunk name: {name}")
            entries.sort(key=lambda item: item[0])
            seen_numbers = set()
            for number, name in entries:
                if strict and number in seen_numbers:
                    raise ValueError(f"Duplicate v3 chunk number: {number}")
                seen_numbers.add(number)
                fpath = os.path.join(path, name)
                size = os.path.getsize(fpath)
                count = _checked_count(size, fpath, record_bytes=self.record_bytes)
                self.chunks.append((self.total, fpath, count))
                self.total += count
            if not self.chunks:
                raise FileNotFoundError(f"No shapes_*.bin in {path}")
        else:
            size = os.path.getsize(path)
            count = _checked_count(size, path, False, self.record_bytes)
            self.chunks = [(0, path, count)]
            self.total = count

    def iter_range(self, start: int, end: int):
        for global_start, fpath, count in self.chunks:
            global_end = global_start + count - 1
            if global_end < start:
                continue
            if global_start > end:
                break

            skip = max(0, start - global_start)
            read_count = min(count - skip, end - max(start, global_start) + 1)
            byte_offset = skip * self.record_bytes

            with open(fpath, "rb") as f:
                f.seek(byte_offset)
                yield from _read_masks(f, read_count, fpath, self.record_bytes,
                                       self.used_bits)


class DatasetIndex:
    """按清单顺序将各分类流拼成一个逻辑全集。"""

    def __init__(self, root):
        self.root = os.path.realpath(root)
        with open(os.path.join(root, "dataset.json"), "r", encoding="utf-8") as f:
            manifest = json.load(f)
        if manifest.get("complete") is not True:
            raise ValueError("Dataset is incomplete")
        if not isinstance(manifest.get("dataset_id"), str) or not manifest["dataset_id"]:
            raise ValueError("Missing dataset_id")
        version = manifest.get("format_version")
        if version == 3:
            self.stride, self.record_bytes, self.used_bits = _v3_properties(manifest)
            expected_total = _decimal_count(manifest.get("count"), "dataset count")
            self.max_n = manifest["max_n"]
        else:
            if manifest.get("encoding") != "u64-le-stride8":
                raise ValueError("Unsupported mask encoding")
            if type(manifest.get("count")) is not int or manifest["count"] < 0:
                raise ValueError("Invalid dataset count")
            self.stride, self.record_bytes, self.used_bits = STRIDE, MASK_SIZE, 64
            self.max_n = STRIDE
            expected_total = manifest["count"]

        self.sources = []  # [(global_start, index)]
        self.chunks = []
        self.total = 0
        try:
            if version == 1:
                self._build_legacy(root)
            elif version in (2, 3):
                self._build_classified(root, manifest, version)
            else:
                raise ValueError(f"Unsupported dataset format version: {version}")
            if self.total != expected_total:
                raise ValueError(f"Dataset count mismatch: {self.total} != {expected_total}")
        except Exception:
            self.close()
            raise

    def _add(self, index):
        start = self.total
        self.sources.append((start, index))
        self.chunks.extend((start + local_start, name, count)
                           for local_start, name, count in index.chunks)
        self.total += index.total

    def _build_legacy(self, root):
        directory = os.path.join(root, "all_fixed")
        archive = directory + ".zip"
        if os.path.isdir(directory):
            self._add(BinIndex(directory))
        elif os.path.isfile(archive):
            self._add(ChunkIndex(archive))
        else:
            raise FileNotFoundError(f"No all_fixed stream in {root}")

    def _build_classified(self, root, manifest, version):
        if manifest.get("storage_layout") != "classified-single-copy":
            raise ValueError("Unsupported storage layout")
        if manifest.get("representative") != "minimum-rotation":
            raise ValueError("Unsupported representative")
        if manifest.get("ordering") != "category-major-parallel-unspecified":
            raise ValueError("Unsupported stream ordering")
        streams = manifest.get("streams")
        if not isinstance(streams, list):
            raise ValueError("Invalid streams")
        seen = set()
        last_order = (-1, 0)
        for stream in streams:
            if not isinstance(stream, dict):
                raise ValueError("Invalid stream entry")
            name = stream.get("path")
            match = STREAM_RE.fullmatch(name) if isinstance(name, str) else None
            if not match:
                raise ValueError(f"Invalid stream path: {name}")
            category, md_text, suffix = match.groups()
            md = int(md_text)
            order = (category == "with_holes", md)
            if (md_text != f"{md:02d}" or not 1 <= md <= self.max_n
                    or order <= last_order or order in seen):
                raise ValueError(f"Duplicate or out-of-order stream: {name}")
            seen.add(order)
            last_order = order
            if type(stream.get("max_dimension")) is not int or stream["max_dimension"] != md:
                raise ValueError(f"Invalid max_dimension for {name}")
            if type(stream.get("has_hole")) is not bool or stream["has_hole"] != order[0]:
                raise ValueError(f"Invalid has_hole for {name}")
            if version == 3:
                stream_count = _decimal_count(stream.get("count"),
                                              f"stream count for {name}", True)
            else:
                if type(stream.get("count")) is not int or stream["count"] <= 0:
                    raise ValueError(f"Invalid stream count for {name}")
                stream_count = stream["count"]
            path = os.path.realpath(os.path.join(root, *name.split("/")))
            if os.path.commonpath((self.root, path)) != self.root:
                raise ValueError(f"Stream path escapes dataset: {name}")
            if not (os.path.isfile(path) if suffix else os.path.isdir(path)):
                raise ValueError(f"Stream path has wrong type or is missing: {name}")
            args = (self.stride, self.record_bytes, self.used_bits, version == 3)
            index = ChunkIndex(path, *args) if suffix else BinIndex(path, *args)
            if index.total != stream_count:
                raise ValueError(f"Stream count mismatch: {name}")
            self._add(index)

    def iter_range(self, start, end):
        for global_start, index in self.sources:
            global_end = global_start + index.total - 1
            if global_end < start:
                continue
            if global_start > end:
                break
            local_start = max(0, start - global_start)
            local_end = min(index.total - 1, end - global_start)
            yield from index.iter_range(local_start, local_end)

    def close(self):
        for _, index in self.sources:
            if hasattr(index, "close"):
                index.close()


def open_index(path: str):
    """自动判断格式，返回带 iter_range 的索引对象。"""
    if os.path.isdir(path) and os.path.isfile(os.path.join(path, "dataset.json")):
        return DatasetIndex(path)
    # 分类流或单个 chunk 被直接打开时，也须采用其 v3 根清单的记录宽度。
    real_path = os.path.realpath(path)
    directory = real_path if os.path.isdir(real_path) else os.path.dirname(real_path)
    while True:
        manifest_path = os.path.join(directory, "dataset.json")
        if os.path.isfile(manifest_path):
            with open(manifest_path, "r", encoding="utf-8") as f:
                manifest = json.load(f)
            if manifest.get("format_version") == 3:
                dataset = DatasetIndex(directory)
                if real_path == os.path.realpath(directory):
                    return dataset
                for _, index in dataset.sources:
                    if real_path == os.path.realpath(index.path):
                        return index
                    if isinstance(index, BinIndex):
                        for _, chunk_path, _ in index.chunks:
                            if real_path == os.path.realpath(chunk_path):
                                return BinIndex(real_path, dataset.stride,
                                                dataset.record_bytes, dataset.used_bits,
                                                True)
                raise ValueError(f"Path is not a v3 dataset stream: {path}")
            break
        parent = os.path.dirname(directory)
        if parent == directory:
            break
        directory = parent
    if path.endswith(".zip"):
        return ChunkIndex(path)
    else:
        return BinIndex(path)


# ================================================================
# 洞检测
# ================================================================

def detect_hole(mask, stride=STRIDE):
    cells = set(mask_to_cells(mask, stride))
    w, h = mask_extent(mask, stride)
    if w <= 2 and h <= 2:
        return False

    from collections import deque
    visited = set()
    queue = deque()
    for c in range(-1, w + 1):
        for r in range(-1, h + 1):
            if c == -1 or c == w or r == -1 or r == h:
                queue.append((c, r))
                visited.add((c, r))

    while queue:
        c, r = queue.popleft()
        for dc, dr in [(0, 1), (0, -1), (1, 0), (-1, 0)]:
            nc, nr = c + dc, r + dr
            if -1 <= nc <= w and -1 <= nr <= h:
                if (nc, nr) not in visited and (nc, nr) not in cells:
                    visited.add((nc, nr))
                    queue.append((nc, nr))

    for c in range(w):
        for r in range(h):
            if (c, r) not in cells and (c, r) not in visited:
                return True
    return False


# ================================================================
# 主逻辑
# ================================================================

def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    path = sys.argv[1]
    mode = "txt"
    start_idx = None
    end_idx = None
    info_only = False
    hole_filter = None

    i = 2
    while i < len(sys.argv):
        a = sys.argv[i]
        if a == "--ascii":       mode = "ascii"
        elif a == "--txt":       mode = "txt"
        elif a == "--info":      info_only = True
        elif a == "--hole":      hole_filter = True
        elif a == "--nohole":    hole_filter = False
        elif a == "--from" and i + 1 < len(sys.argv):
            start_idx = int(sys.argv[i + 1]) - 1; i += 1
        elif a == "--to" and i + 1 < len(sys.argv):
            end_idx = int(sys.argv[i + 1]) - 1; i += 1
        elif a == "--limit" and i + 1 < len(sys.argv):
            start_idx = 0; end_idx = int(sys.argv[i + 1]) - 1; i += 1
        i += 1

    idx = open_index(path)
    try:
        if info_only:
            size = idx.total * idx.record_bytes
            print(f"File: {os.path.basename(path)}")
            print(f"Size: {size:,} bytes ({size / 1e6:.2f} MB)")
            print(f"Shapes: {idx.total:,}")
            if len(idx.chunks) > 1:
                print(f"Chunks: {len(idx.chunks)}")
            return

        if start_idx is None:
            start_idx = 0
        if end_idx is None:
            end_idx = idx.total - 1

        loaded_chunks = sum(1 for global_start, _, count in idx.chunks
                            if global_start + count - 1 >= start_idx and global_start <= end_idx)

        print(f"# File: {os.path.basename(path)}")
        if hole_filter is not None:
            print(f"# Filter: {'has_hole' if hole_filter else 'no_hole'}")
        print(f"# Total: {idx.total:,}  Range: [{start_idx + 1}, {end_idx + 1}]")
        print(f"# Chunks to decompress: {loaded_chunks}")
        print()

        displayed = 0
        for global_i, mask in enumerate(idx.iter_range(start_idx, end_idx)):
            idx_num = start_idx + global_i + 1
            if hole_filter is not None and detect_hole(mask, idx.stride) != hole_filter:
                continue
            w, h = mask_extent(mask, idx.stride)
            size = mask_popcount(mask)
            if mode == "ascii":
                print(f"--- Shape {idx_num} (size={size}, {w}x{h}) ---")
                print(shape_to_ascii(mask, idx.stride))
                print()
            else:
                print(shape_to_text(mask, idx.stride))
            displayed += 1

        if displayed == 0:
            print("(no matching shapes)")
    finally:
        if hasattr(idx, "close"):
            idx.close()


if __name__ == "__main__":
    main()
