"""
从 Fixed mask 二进制文件中提取和可视化多联骨牌形状。

二进制格式: 每个形状 8 字节 u64 little-endian
位图编码: 行优先, stride=8, 左上角对齐 (0,0)

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
import os
import re
import struct
import zipfile
import json

STRIDE = 8
STRIDE_SHIFT = 3
MASK_SIZE = 8
CHUNK_SIZE = 10_000_000  # 与 Rust export.rs 一致
READ_BATCH_SIZE = 256 * 1024


# ================================================================
# Mask 操作
# ================================================================

def mask_to_cells(mask):
    cells = []
    while mask:
        lsb = mask & -mask
        bit = (lsb.bit_length() - 1)
        r = bit >> STRIDE_SHIFT
        c = bit & (STRIDE - 1)
        cells.append((c, r))
        mask ^= lsb
    return cells


def mask_extent(mask):
    cells = mask_to_cells(mask)
    if not cells:
        return (0, 0)
    return (max(c for c, r in cells) + 1, max(r for c, r in cells) + 1)


def mask_popcount(mask):
    return mask.bit_count()


def shape_to_ascii(mask):
    cells = set(mask_to_cells(mask))
    w, h = mask_extent(mask)
    grid = [["  " for _ in range(w)] for _ in range(h)]
    for c, r in cells:
        grid[h - 1 - r][c] = "██"
    return "\n".join("".join(row) for row in grid)


def shape_to_text(mask):
    cells = mask_to_cells(mask)
    cells_str = " ".join(f"({c},{r})" for c, r in cells)
    size = mask_popcount(mask)
    w, h = mask_extent(mask)
    return f"[size={size} {w}x{h}] {cells_str}"


# ================================================================
# Chunk 索引
# ================================================================

CHUNK_RE = re.compile(r"shapes_(\d{6})\.bin$")
STREAM_RE = re.compile(r"(no_holes|with_holes)/n(\d{2})_fixed(\.zip)?$")


def _checked_count(size, name, chunk=True):
    if size % MASK_SIZE:
        raise ValueError(f"Mask data length is not divisible by 8: {name}")
    count = size // MASK_SIZE
    if chunk and count > CHUNK_SIZE:
        raise ValueError(f"Chunk exceeds {CHUNK_SIZE} masks: {name}")
    return count


def _read_masks(file, count, name):
    """以固定大小批量解码，短读必须报错。"""
    batch_count = READ_BATCH_SIZE // MASK_SIZE
    while count:
        take = min(count, batch_count)
        expected = take * MASK_SIZE
        data = file.read(expected)
        if len(data) != expected:
            raise ValueError(f"Short mask read: {name}")
        for (mask,) in struct.iter_unpack("<Q", data):
            yield mask
        count -= take


class ChunkIndex:
    """zip 内多个 chunk 文件的索引，支持按全局 mask 编号定位到具体 chunk 和偏移。"""

    def __init__(self, zip_path: str):
        self.chunks = []  # [(start_global_idx, entry_name, file_size)]
        self.total = 0
        self.path = zip_path
        self._build(zip_path)

    def _build(self, zip_path: str):
        with zipfile.ZipFile(zip_path, "r") as zf:
            entries = []
            for name in zf.namelist():
                m = CHUNK_RE.search(name)
                if m:
                    info = zf.getinfo(name)
                    entries.append((int(m.group(1)), name, info.file_size))

            if not entries:
                # 兼容旧格式：只有一个 shapes_0001.bin 或无编号
                for name in zf.namelist():
                    if name.endswith(".bin"):
                        info = zf.getinfo(name)
                        entries.append((1, name, info.file_size))
                        break

            if not entries:
                raise FileNotFoundError(f"No .bin found in {zip_path}")

            entries.sort(key=lambda x: x[0])
            seen = set()
            for _, name, size in entries:
                if name in seen:
                    raise ValueError(f"Duplicate ZIP entry: {name}")
                seen.add(name)
                count = _checked_count(size, name, CHUNK_RE.search(name) is not None)
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
            byte_offset = skip * MASK_SIZE
            # 按需打开 ZIP；清单扫描及 --info 不保留文件句柄。
            with zipfile.ZipFile(self.path, "r") as zf:
                with zf.open(name, "r") as f:
                    f.seek(byte_offset)
                    yield from _read_masks(f, read_count, name)

    def close(self):
        # iter_range 中的 with 块在迭代结束或关闭时释放句柄。
        pass


# ================================================================
# .bin 单文件读取（也支持 chunk 化目录）
# ================================================================

class BinIndex:
    """单 .bin 文件的索引（chunk 化目录或裸文件）。"""

    def __init__(self, path: str):
        self.path = path
        if os.path.isdir(path):
            self.chunks = []
            self.total = 0
            for name in sorted(os.listdir(path)):
                m = CHUNK_RE.match(name)
                if m:
                    fpath = os.path.join(path, name)
                    size = os.path.getsize(fpath)
                    count = _checked_count(size, fpath)
                    self.chunks.append((self.total, fpath, count))
                    self.total += count
            if not self.chunks:
                raise FileNotFoundError(f"No shapes_*.bin in {path}")
        else:
            size = os.path.getsize(path)
            count = _checked_count(size, path, False)
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
            byte_offset = skip * MASK_SIZE

            with open(fpath, "rb") as f:
                f.seek(byte_offset)
                yield from _read_masks(f, read_count, fpath)


class DatasetIndex:
    """按清单顺序将各分类流拼成一个逻辑全集。"""

    def __init__(self, root):
        self.root = os.path.realpath(root)
        with open(os.path.join(root, "dataset.json"), "r", encoding="utf-8") as f:
            manifest = json.load(f)
        if manifest.get("complete") is not True:
            raise ValueError("Dataset is incomplete")
        if manifest.get("encoding") != "u64-le-stride8":
            raise ValueError("Unsupported mask encoding")
        if not isinstance(manifest.get("dataset_id"), str) or not manifest["dataset_id"]:
            raise ValueError("Missing dataset_id")
        if type(manifest.get("count")) is not int or manifest["count"] < 0:
            raise ValueError("Invalid dataset count")

        self.sources = []  # [(global_start, index)]
        self.chunks = []
        self.total = 0
        version = manifest.get("format_version")
        try:
            if version == 1:
                self._build_legacy(root)
            elif version == 2:
                self._build_classified(root, manifest)
            else:
                raise ValueError(f"Unsupported dataset format version: {version}")
            if self.total != manifest["count"]:
                raise ValueError(f"Dataset count mismatch: {self.total} != {manifest['count']}")
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

    def _build_classified(self, root, manifest):
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
            if not 1 <= md <= STRIDE or order <= last_order or order in seen:
                raise ValueError(f"Duplicate or out-of-order stream: {name}")
            seen.add(order)
            last_order = order
            if type(stream.get("max_dimension")) is not int or stream["max_dimension"] != md:
                raise ValueError(f"Invalid max_dimension for {name}")
            if type(stream.get("has_hole")) is not bool or stream["has_hole"] != order[0]:
                raise ValueError(f"Invalid has_hole for {name}")
            if type(stream.get("count")) is not int or stream["count"] <= 0:
                raise ValueError(f"Invalid stream count for {name}")
            path = os.path.realpath(os.path.join(root, *name.split("/")))
            if os.path.commonpath((self.root, path)) != self.root:
                raise ValueError(f"Stream path escapes dataset: {name}")
            if not (os.path.isfile(path) if suffix else os.path.isdir(path)):
                raise ValueError(f"Stream path has wrong type or is missing: {name}")
            index = ChunkIndex(path) if suffix else BinIndex(path)
            if index.total != stream["count"]:
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
    if path.endswith(".zip"):
        return ChunkIndex(path)
    else:
        return BinIndex(path)


# ================================================================
# 洞检测
# ================================================================

def detect_hole(mask):
    cells = set(mask_to_cells(mask))
    w, h = mask_extent(mask)
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
            size = idx.total * MASK_SIZE
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
            if hole_filter is not None and detect_hole(mask) != hole_filter:
                continue
            w, h = mask_extent(mask)
            size = mask_popcount(mask)
            if mode == "ascii":
                print(f"--- Shape {idx_num} (size={size}, {w}x{h}) ---")
                print(shape_to_ascii(mask))
                print()
            else:
                print(shape_to_text(mask))
            displayed += 1

        if displayed == 0:
            print("(no matching shapes)")
    finally:
        if hasattr(idx, "close"):
            idx.close()


if __name__ == "__main__":
    main()
