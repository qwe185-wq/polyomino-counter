"""
从 polyomino-landmarks 各 n 值的 landmarks.parquet 读取 global_index，
用 ChunkIndex 从 all_fixed.zip 批量提取形状，
按 n 值分目录输出 rooms.bin + rooms.json 到 Backrooms 项目。
也支持直接查看已提取的 rooms.bin。

用法:
  # 批量提取所有 n 值（默认）
  python extract_landmarks.py

  # 只提取指定 n 值
  python extract_landmarks.py --n 3
  python extract_landmarks.py --n 5 --n 6

  # 查看模式（从已提取的 rooms.bin 读取）
  python extract_landmarks.py --info                  # 全部汇总
  python extract_landmarks.py --info --n 5            # 只看 n=5
  python extract_landmarks.py --txt --n 4 --limit 5   # n=4 前 5 个形状
  python extract_landmarks.py --ascii --n 3           # n=3 的图形
  python extract_landmarks.py --txt --n 6 --hole --limit 5
"""

import json
import os
import struct
import sys
import io

if sys.stdout.encoding != 'utf-8':
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding='utf-8', errors='replace')

from read_shapes import (ChunkIndex, MASK_SIZE,
                         mask_to_cells, mask_extent, mask_popcount, detect_hole,
                         shape_to_ascii, shape_to_text)

# ================================================================
# 配置
# ================================================================

PARQUET_BASE = "D:/git/polyomino-landmarks/output"
ZIP_PATH = "D:/git/room-count/rust/output_n6/all_fixed.zip"
OUTPUT_BASE = "D:/git/Backrooms/assets/rooms"

# 每个 n 值的配置: (n标签, parquet子路径, 输出子目录)
EXTRACTION_CONFIGS = [
    ("n3", "n3/k4/landmarks.parquet",     "n3"),
    ("n4", "n4/k100/landmarks.parquet",   "n4"),
    ("n5", "n5/k1000/landmarks.parquet",  "n5"),
    ("n6", "n6_combined/landmarks.parquet", "n6"),
]


# ================================================================
# 提取核心
# ================================================================

def extract_one(label, parquet_relpath, output_subdir):
    """提取单个 n 值的全部形状。"""
    parquet_path = os.path.join(PARQUET_BASE, parquet_relpath)
    output_dir = os.path.join(OUTPUT_BASE, output_subdir)

    # ── 1. 读取 parquet ──
    print(f"[{label}] 读取 {parquet_relpath}...")
    import pyarrow.parquet as pq
    table = pq.read_table(parquet_path)
    df = table.to_pandas()

    n = len(df)
    print(f"  → {n} 个 landmark")
    if n == 0:
        print(f"  → 跳过（空）")
        return

    global_indices = df["global_index"].tolist()
    selection_orders = df["selection_order"].tolist()
    mask_from_parquet = df["mask"].tolist()

    # ── 2. 从 zip 提取 ──
    print(f"  从 all_fixed.zip 提取...")
    idx = ChunkIndex(ZIP_PATH)

    sorted_pairs = sorted(enumerate(global_indices), key=lambda x: x[1])
    sorted_orig_indices = [p[0] for p in sorted_pairs]
    sorted_global_indices = [p[1] for p in sorted_pairs]

    chunk_groups = {}
    for pos, gi in enumerate(sorted_global_indices):
        for chunk_start, chunk_name, chunk_count in idx.chunks:
            if chunk_start <= gi < chunk_start + chunk_count:
                chunk_groups.setdefault(chunk_name, []).append((pos, gi))
                break

    extracted_masks = [0] * n
    for chunk_name, requests in chunk_groups.items():
        chunk_start = None
        for cs, cn, cc in idx.chunks:
            if cn == chunk_name:
                chunk_start = cs
                break
        requests_sorted = sorted(requests, key=lambda x: x[1])
        with idx._zf.open(chunk_name, "r") as f:
            for pos, gi in requests_sorted:
                offset = (gi - chunk_start) * MASK_SIZE
                f.seek(offset)
                data = f.read(MASK_SIZE)
                extracted_masks[pos] = struct.unpack("<Q", data)[0]
    idx.close()

    # ── 3. 验证 & 计算元数据 ──
    rooms = []
    mismatches = 0
    total_cells = 0
    holes_count = 0

    for orig_i in range(n):
        si = sorted_orig_indices.index(orig_i)
        mask = extracted_masks[si]
        parquet_mask = int(mask_from_parquet[orig_i])

        if mask != parquet_mask:
            mismatches += 1
            if mismatches <= 3:
                print(f"  ⚠ 不匹配 [{orig_i}] global={global_indices[orig_i]}: "
                      f"zip={mask:016x} vs pq={parquet_mask:016x}")

        w, h = mask_extent(mask)
        size = mask_popcount(mask)
        hole = detect_hole(mask)
        total_cells += size
        if hole:
            holes_count += 1

        rooms.append({
            "selection_order": int(selection_orders[orig_i]),
            "global_index": int(global_indices[orig_i]),
            "mask": mask,
            "width": w, "height": h, "size": size, "hole": hole
        })

    ok = "✓" if mismatches == 0 else f"⚠ {mismatches} 不匹配"
    avg_size = total_cells / n if n > 0 else 0
    print(f"  {ok}  平均 size={avg_size:.1f}  有洞 {holes_count}/{n} "
          f"({holes_count/n*100:.1f}%)")

    # ── 4. 写入 ──
    os.makedirs(output_dir, exist_ok=True)

    bin_path = os.path.join(output_dir, "rooms.bin")
    with open(bin_path, "wb") as f:
        for room in rooms:
            f.write(struct.pack("<Q", room["mask"]))

    json_data = {
        "label": label,
        "count": n,
        "source": f"polyomino-landmarks {label} k-means++",
        "avg_size": round(avg_size, 1),
        "holes_count": holes_count,
        "format": "rooms.bin: u64 LE bitmask × count, stride=8, top-left aligned",
        "rooms": [
            {k: r[k] for k in ["selection_order", "global_index",
                                "width", "height", "size", "hole"]}
            for r in rooms
        ]
    }
    json_path = os.path.join(output_dir, "rooms.json")
    with open(json_path, "w", encoding="utf-8") as f:
        json.dump(json_data, f, ensure_ascii=False, indent=2)

    sizes = [r["size"] for r in rooms]
    print(f"  → {output_subdir}/rooms.bin: {os.path.getsize(bin_path):,} B  "
          f"rooms.json: {os.path.getsize(json_path):,} B")
    print(f"     size [{min(sizes)}~{max(sizes)}]  "
          f"w×h [{min(r['width'] for r in rooms)}~{max(r['width'] for r in rooms)}"
          f"×{min(r['height'] for r in rooms)}~{max(r['height'] for r in rooms)}]")


def do_extract(targets=None):
    """targets: None=全部, 否则为 ['n3','n5'] 等"""
    for label, parquet_rel, subdir in EXTRACTION_CONFIGS:
        if targets and label not in targets:
            continue
        print()
        extract_one(label, parquet_rel, subdir)
    print(f"\n✓ 全部完成 → {OUTPUT_BASE}/")


# ================================================================
# 查看模式
# ================================================================

def load_rooms_for_label(label):
    """加载某个 n 值的房间数据。"""
    dir_map = {cfg[0]: cfg[2] for cfg in EXTRACTION_CONFIGS}
    if label not in dir_map:
        print(f"错误: 未知 label '{label}'，可选: {list(dir_map.keys())}")
        sys.exit(1)
    subdir = dir_map[label]
    bin_path = os.path.join(OUTPUT_BASE, subdir, "rooms.bin")
    json_path = os.path.join(OUTPUT_BASE, subdir, "rooms.json")

    if not os.path.exists(json_path):
        print(f"错误: 找不到 {json_path}，请先运行提取模式")
        sys.exit(1)

    with open(json_path, "r", encoding="utf-8") as f:
        meta = json.load(f)

    rooms = meta["rooms"]
    with open(bin_path, "rb") as f:
        for room in rooms:
            data = f.read(8)
            room["mask"] = struct.unpack("<Q", data)[0]

    return rooms, meta


def do_info_all():
    """显示所有 n 值的汇总。"""
    grand_total = 0
    grand_holes = 0
    for label, _, _ in EXTRACTION_CONFIGS:
        json_path = os.path.join(OUTPUT_BASE, label.replace("n", "n"), "rooms.json")  # subdir matches label
        # Actually let me fix this - subdir is like 'n3', 'n4' etc
        pass

    # Simpler approach
    print(f"文件源: {OUTPUT_BASE}/\n")
    for label, _, subdir in EXTRACTION_CONFIGS:
        json_path = os.path.join(OUTPUT_BASE, subdir, "rooms.json")
        if not os.path.exists(json_path):
            print(f"  {subdir}/  未提取")
            continue
        with open(json_path, "r", encoding="utf-8") as f:
            meta = json.load(f)
        n = meta["count"]
        h = meta["holes_count"]
        s = meta["avg_size"]
        grand_total += n
        grand_holes += h
        bin_size = os.path.getsize(os.path.join(OUTPUT_BASE, subdir, "rooms.bin"))
        print(f"  {subdir}/  {n:5d} 个  avg_size={s:4.1f}  "
              f"有洞={h}/{n}  bin={bin_size:,} B")
    print(f"\n  合计: {grand_total:,} 个, 有洞 {grand_holes}")


def do_info_label(label, rooms, meta):
    """显示单个 n 值的摘要。"""
    print(f"文件: {label}/rooms.bin + rooms.json")
    print(f"来源: {meta['source']}")
    print(f"形状数: {meta['count']:,}")
    print(f"平均 size: {meta['avg_size']}")
    h = meta["holes_count"]
    n = meta["count"]
    print(f"有洞: {h}/{n} ({h/n*100:.1f}%)" if n > 0 else "")
    bin_path = os.path.join(OUTPUT_BASE, label.replace("n", "n"), "rooms.bin")
    json_path = os.path.join(OUTPUT_BASE, label.replace("n", "n"), "rooms.json")
    # Fix path to use subdir
    dir_map = {cfg[0]: cfg[2] for cfg in EXTRACTION_CONFIGS}
    subdir = dir_map[label]
    bin_path = os.path.join(OUTPUT_BASE, subdir, "rooms.bin")
    json_path = os.path.join(OUTPUT_BASE, subdir, "rooms.json")
    print(f"bin: {os.path.getsize(bin_path):,} B  json: {os.path.getsize(json_path):,} B")

    if n == 0:
        return
    sizes = [r["size"] for r in rooms]
    from collections import Counter
    size_dist = Counter(sizes)
    print(f"\nsize [{min(sizes)}~{max(sizes)}] 分布:")
    for sz, cnt in sorted(size_dist.items()):
        bar = "█" * max(1, cnt * 40 // max(size_dist.values()))
        print(f"  {sz:2d}: {cnt:4d} {bar}")


def do_view_label(label, rooms, meta, mode, start_idx, end_idx, hole_filter):
    """查看某个 n 值的形状。"""
    n = meta["count"]
    if start_idx is None:
        start_idx = 0
    if end_idx is None:
        end_idx = n - 1

    print(f"# {label}  |  {meta['source']}")
    if hole_filter is not None:
        print(f"# 筛选: {'有洞' if hole_filter else '无洞'}")
    print(f"# 总数: {n:,}  范围: [{start_idx + 1}, {end_idx + 1}]")
    print()

    displayed = 0
    for i in range(start_idx, min(end_idx + 1, n)):
        room = rooms[i]
        mask = room["mask"]

        if hole_filter is not None and room["hole"] != hole_filter:
            continue

        idx_num = i + 1
        w, h = room["width"], room["height"]
        size = room["size"]
        hole = room["hole"]

        if mode == "ascii":
            print(f"--- {label} 房间 {idx_num} "
                  f"(sel={room['selection_order']}, "
                  f"size={size}, {w}x{h}, "
                  f"{'有洞' if hole else '无洞'}) ---")
            print(shape_to_ascii(mask))
            print()
        else:
            print(f"[{idx_num}] sel={room['selection_order']} "
                  f"global={room['global_index']} "
                  f"size={size} {w}x{h} "
                  f"{'H' if hole else '.'} "
                  f"{shape_to_text(mask)}")

        displayed += 1

    if displayed == 0:
        print("(无匹配形状)")
    print(f"\n# 显示: {displayed}")


# ================================================================
# CLI
# ================================================================

def main():
    # 解析参数
    targets = []
    view_mode = False
    info_only = False
    txt_mode = False
    ascii_mode = False
    start_idx = None
    end_idx = None
    hole_filter = None

    i = 1
    while i < len(sys.argv):
        a = sys.argv[i]
        if a == "--n" and i + 1 < len(sys.argv):
            i += 1
            val = sys.argv[i]
            targets.append(val if val.startswith('n') else f'n{val}')
        elif a in ("--info", "--txt", "--ascii"):
            view_mode = True
            if a == "--info":
                info_only = True
            elif a == "--txt":
                txt_mode = True
            elif a == "--ascii":
                ascii_mode = True
        elif a == "--hole":
            view_mode = True
            hole_filter = True
        elif a == "--nohole":
            view_mode = True
            hole_filter = False
        elif a == "--from" and i + 1 < len(sys.argv):
            view_mode = True
            start_idx = int(sys.argv[i + 1]) - 1
            i += 1
        elif a == "--to" and i + 1 < len(sys.argv):
            view_mode = True
            end_idx = int(sys.argv[i + 1]) - 1
            i += 1
        elif a == "--limit" and i + 1 < len(sys.argv):
            view_mode = True
            start_idx = 0
            end_idx = int(sys.argv[i + 1]) - 1
            i += 1
        i += 1

    # ── 提取模式 ──
    if not view_mode:
        do_extract(targets if targets else None)
        return

    # ── 查看模式 ──
    if not targets:
        # 没有指定 --n，默认显示全部汇总
        targets = [cfg[0] for cfg in EXTRACTION_CONFIGS]

    mode = "ascii" if ascii_mode else "txt"

    if info_only and len(targets) > 1:
        do_info_all()
        return

    for label in targets:
        rooms, meta = load_rooms_for_label(label)

        if len(targets) > 1:
            print(f"\n{'='*60}")
            print(f"  {label}")
            print(f"{'='*60}")

        if info_only:
            do_info_label(label, rooms, meta)
        else:
            do_view_label(label, rooms, meta, mode, start_idx, end_idx, hole_filter)


if __name__ == "__main__":
    main()
