"""分类数据集与旧导出格式的读取契约。"""

import json
import contextlib
import io
import struct
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch

from read_shapes import (main, open_index, mask_to_cells, mask_extent,
                         shape_to_ascii, shape_to_text, detect_hole)


def masks(*values):
    return b"".join(struct.pack("<Q", value) for value in values)


class ReadShapesTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write_stream(self, name, values, zipped=False):
        path = self.root / name
        if zipped:
            path = path.with_suffix(".zip")
            path.parent.mkdir(parents=True, exist_ok=True)
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr(f"{Path(name).name}/shapes_000001.bin", masks(*values))
            return name + ".zip"
        path.mkdir(parents=True)
        (path / "shapes_000001.bin").write_bytes(masks(*values))
        return name

    def write_manifest(self, entries, **changes):
        manifest = {
            "format_version": 2,
            "storage_layout": "classified-single-copy",
            "dataset_id": "test-dataset",
            "encoding": "u64-le-stride8",
            "representative": "minimum-rotation",
            "ordering": "category-major-parallel-unspecified",
            "count": sum(stream["count"] for stream in entries),
            "complete": True,
            "streams": entries,
        }
        manifest.update(changes)
        (self.root / "dataset.json").write_text(json.dumps(manifest), encoding="utf-8")

    def write_v3_stream(self, name, values, record_bytes, zipped=False):
        data = b"".join(value.to_bytes(record_bytes, "little") for value in values)
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if zipped:
            path = path.with_suffix(".zip")
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr(f"{Path(name).name}/shapes_000001.bin", data)
            return name + ".zip"
        path.mkdir()
        (path / "shapes_000001.bin").write_bytes(data)
        return name

    def write_v3_manifest(self, entries, n, **changes):
        stride = max(8, n)
        manifest = {
            "format_version": 3,
            "storage_layout": "classified-single-copy",
            "dataset_id": "test-v3",
            "encoding": "uint-le-fixed",
            "representative": "minimum-rotation",
            "equivalence": "one-sided",
            "category_dimension": "exact-max-bounding-box",
            "ordering": "category-major-parallel-unspecified",
            "max_n": n,
            "stride": stride,
            "record_bytes": max(8, (n * stride + 7) // 8),
            "count": str(sum(int(stream["count"]) for stream in entries)),
            "complete": True,
            "streams": entries,
        }
        manifest.update(changes)
        (self.root / "dataset.json").write_text(json.dumps(manifest), encoding="utf-8")

    def test_classified_root_joins_streams_in_manifest_order_across_boundaries(self):
        streams = [
            {"path": self.write_stream("no_holes/n01_fixed", [1, 2]), "max_dimension": 1, "has_hole": False, "count": 2},
            {"path": self.write_stream("no_holes/n02_fixed", [3], zipped=True), "max_dimension": 2, "has_hole": False, "count": 1},
            {"path": self.write_stream("with_holes/n03_fixed", [4, 5]), "max_dimension": 3, "has_hole": True, "count": 2},
        ]
        self.write_manifest(streams)
        index = open_index(str(self.root))
        try:
            self.assertEqual(index.total, 5)
            self.assertEqual(list(index.iter_range(1, 3)), [2, 3, 4])
            self.assertEqual(list(index.iter_range(0, 4)), [1, 2, 3, 4, 5])
            self.assertEqual(len(index.chunks), 3)
        finally:
            index.close()
        output = io.StringIO()
        with patch.object(sys, "argv", ["read_shapes.py", str(self.root), "--from", "2", "--to", "4"]):
            with contextlib.redirect_stdout(output):
                main()
        self.assertIn("# Chunks to decompress: 3", output.getvalue())
        self.assertEqual(output.getvalue().count("[size="), 3)

    def test_legacy_v1_root_and_standalone_inputs(self):
        old = self.root / "all_fixed"
        old.mkdir()
        (old / "shapes_000001.bin").write_bytes(masks(7, 8))
        (self.root / "dataset.json").write_text(json.dumps({
            "format_version": 1, "count": 2, "complete": True,
            "encoding": "u64-le-stride8", "dataset_id": "old",
        }), encoding="utf-8")
        for path in (self.root, old, old / "shapes_000001.bin"):
            index = open_index(str(path))
            try:
                self.assertEqual(list(index.iter_range(0, 1)), [7, 8])
            finally:
                if hasattr(index, "close"):
                    index.close()
        archive = self.root / "old.zip"
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr("all_fixed/shapes_000001.bin", masks(7, 8))
        index = open_index(str(archive))
        try:
            self.assertEqual(list(index.iter_range(0, 1)), [7, 8])
        finally:
            index.close()

    def test_rejects_incomplete_unsupported_encoding_and_wrong_counts(self):
        path = self.write_stream("no_holes/n01_fixed", [1, 2])
        stream = {"path": path, "max_dimension": 1, "has_hole": False, "count": 2}
        for changes in ({"complete": False}, {"encoding": "u32-le"}, {"count": 3},
                        {"streams": [{**stream, "count": 3}]}):
            with self.subTest(changes=changes):
                self.write_manifest([stream], **changes)
                with self.assertRaises(ValueError):
                    open_index(str(self.root))

    def test_rejects_bad_stream_paths_duplicates_and_unaligned_chunks(self):
        path = self.write_stream("no_holes/n01_fixed", [1])
        stream = {"path": path, "max_dimension": 1, "has_hole": False, "count": 1}
        for bad_path in ("../escape", "/absolute", "no_holes/../n01_fixed", "with_holes/n01_fixed"):
            with self.subTest(path=bad_path):
                self.write_manifest([{**stream, "path": bad_path}])
                with self.assertRaises(ValueError):
                    open_index(str(self.root))
        self.write_manifest([stream, stream])
        with self.assertRaises(ValueError):
            open_index(str(self.root))
        self.write_manifest([stream])
        (self.root / path / "shapes_000001.bin").write_bytes(b"abc")
        with self.assertRaises(ValueError):
            open_index(str(self.root))

    def test_zip_is_lazy_and_close_releases_archive(self):
        archive = self.root / "standalone.zip"
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr("shapes_000001.bin", masks(11, 12))
        index = open_index(str(archive))
        self.assertEqual(list(index.iter_range(0, 0)), [11])
        index.close()
        self.assertEqual(list(index.iter_range(1, 1)), [12])
        index.close()

    def test_zip_batch_boundary_and_bad_zip_length(self):
        values = list(range(32770))  # 跨过 256 KiB 解码批次。
        archive = self.root / "large.zip"
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr("shapes_000001.bin", masks(*values))
        index = open_index(str(archive))
        try:
            self.assertEqual(list(index.iter_range(32766, 32769)), values[32766:])
            self.assertEqual(list(index.iter_range(0, 32769)), values)
        finally:
            index.close()
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr("shapes_000001.bin", b"123")
        with self.assertRaises(ValueError):
            open_index(str(archive))

    def test_truncated_bin_after_indexing_raises_on_read(self):
        path = self.root / "shapes.bin"
        path.write_bytes(masks(1, 2, 3))
        index = open_index(str(path))
        path.write_bytes(masks(1))
        with self.assertRaises(ValueError):
            list(index.iter_range(0, 2))

    def test_legacy_v1_root_can_use_all_fixed_zip(self):
        archive = self.root / "all_fixed.zip"
        with zipfile.ZipFile(archive, "w") as out:
            out.writestr("all_fixed/shapes_000001.bin", masks(21, 22))
        (self.root / "dataset.json").write_text(json.dumps({
            "format_version": 1, "count": 2, "complete": True,
            "encoding": "u64-le-stride8", "dataset_id": "old-zip",
        }), encoding="utf-8")
        index = open_index(str(self.root))
        try:
            self.assertEqual(list(index.iter_range(0, 1)), [21, 22])
        finally:
            index.close()

    def test_v3_n9_crosses_64_bits_and_direct_stream_uses_manifest(self):
        # 第 8 行首位落在 bit 72；3x3 环形的中心空格是一个洞。
        ring = sum(1 << (r * 9 + c) for r in range(3) for c in range(3)
                   if (r, c) != (1, 1))
        high = (1 << 72) | 1
        streams = [
            {"path": self.write_v3_stream("no_holes/n09_fixed", [high], 11),
             "max_dimension": 9, "has_hole": False, "count": "1"},
            {"path": self.write_v3_stream("with_holes/n09_fixed", [ring], 11,
                                           zipped=True),
             "max_dimension": 9, "has_hole": True, "count": "1"},
        ]
        self.write_v3_manifest(streams, 9)
        for path in (self.root, self.root / streams[0]["path"],
                     self.root / streams[0]["path"] / "shapes_000001.bin",
                     self.root / streams[1]["path"]):
            index = open_index(str(path))
            self.assertEqual(index.stride, 9)
            self.assertEqual(index.record_bytes, 11)
            self.assertEqual(list(index.iter_range(0, index.total - 1)),
                             [high, ring] if path == self.root else
                             ([ring] if str(path).endswith(".zip") else [high]))
            if hasattr(index, "close"):
                index.close()
        self.assertEqual(mask_to_cells(high, 9), [(0, 0), (0, 8)])
        self.assertEqual(mask_extent(high, 9), (1, 9))
        self.assertTrue(detect_hole(ring, 9))
        self.assertIn("3x3", shape_to_text(ring, 9))
        self.assertIn("██  ██", shape_to_ascii(ring, 9))
        output = io.StringIO()
        with patch.object(sys, "argv", ["read_shapes.py", str(self.root), "--info"]):
            with contextlib.redirect_stdout(output):
                main()
        self.assertIn("22 bytes", output.getvalue())

    def test_v3_n17_batch_and_stream_boundary(self):
        width = 37
        first = list(range(7100))
        high = (1 << 272) | 1
        streams = [
            {"path": self.write_v3_stream("no_holes/n17_fixed", first, width),
             "max_dimension": 17, "has_hole": False, "count": str(len(first))},
            {"path": self.write_v3_stream("with_holes/n17_fixed", [high], width,
                                           zipped=True),
             "max_dimension": 17, "has_hole": True, "count": "1"},
        ]
        self.write_v3_manifest(streams, 17)
        index = open_index(str(self.root))
        try:
            self.assertEqual(index.record_bytes, width)
            self.assertEqual(list(index.iter_range(7082, 7100)), first[7082:] + [high])
            self.assertEqual(list(index.iter_range(0, 7100)), first + [high])
        finally:
            index.close()

    def test_v3_n100_path_has_no_small_n_limit(self):
        name = self.write_v3_stream("no_holes/n100_fixed", [1 << 9999], 1250)
        stream = {"path": name, "max_dimension": 100,
                  "has_hole": False, "count": "1"}
        self.write_v3_manifest([stream], 100)
        index = open_index(str(self.root))
        try:
            self.assertEqual(index.stride, 100)
            self.assertEqual(list(index.iter_range(0, 0)), [1 << 9999])
        finally:
            index.close()

    def test_v3_chunk_numbers_cross_six_digit_boundary_in_bin_and_zip(self):
        for zipped in (False, True):
            with self.subTest(zipped=zipped):
                name = "no_holes/n09_fixed"
                data_first = (11).to_bytes(11, "little")
                data_second = (22).to_bytes(11, "little")
                if zipped:
                    path = self.root / (name + ".zip")
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with zipfile.ZipFile(path, "w") as out:
                        # 故意逆序写入，读取须按十进制编号排序。
                        out.writestr("n09_fixed/shapes_1000000.bin", data_second)
                        out.writestr("n09_fixed/shapes_999999.bin", data_first)
                    stream_name = name + ".zip"
                else:
                    path = self.root / name
                    path.mkdir(parents=True, exist_ok=True)
                    (path / "shapes_1000000.bin").write_bytes(data_second)
                    (path / "shapes_999999.bin").write_bytes(data_first)
                    stream_name = name
                stream = {"path": stream_name, "max_dimension": 9,
                          "has_hole": False, "count": "2"}
                self.write_v3_manifest([stream], 9)
                index = open_index(str(self.root))
                try:
                    self.assertEqual(list(index.iter_range(0, 1)), [11, 22])
                    self.assertEqual(len(index.chunks), 2)
                finally:
                    index.close()
                if zipped:
                    path.unlink()
                else:
                    for chunk in path.iterdir():
                        chunk.unlink()
                    path.rmdir()

    def test_v3_zip_rejects_duplicate_numbers_and_malformed_bin(self):
        name = "no_holes/n09_fixed.zip"
        path = self.root / name
        path.parent.mkdir(parents=True)
        stream = {"path": name, "max_dimension": 9,
                  "has_hole": False, "count": "1"}
        self.write_v3_manifest([stream], 9)
        for second_name in ("other/shapes_000001.bin", "n09_fixed/other.bin"):
            with self.subTest(second_name=second_name):
                with zipfile.ZipFile(path, "w") as out:
                    out.writestr("n09_fixed/shapes_000001.bin", (1).to_bytes(11, "little"))
                    out.writestr(second_name, (2).to_bytes(11, "little"))
                with self.assertRaises(ValueError):
                    open_index(str(self.root))

    def test_old_zip_fallback_accepts_short_chunk_number(self):
        path = self.root / "legacy.zip"
        with zipfile.ZipFile(path, "w") as out:
            out.writestr("shapes_0001.bin", masks(7))
        index = open_index(str(path))
        self.assertEqual(list(index.iter_range(0, 0)), [7])
        index.close()

    def test_v3_rejects_wrong_width_counts_and_unused_high_bits(self):
        name = self.write_v3_stream("no_holes/n09_fixed", [1], 11)
        stream = {"path": name, "max_dimension": 9, "has_hole": False, "count": "1"}
        for change in ({"stride": 8}, {"record_bytes": 12}, {"count": 1},
                       {"count": "01"}, {"count": "18446744073709551616"},
                       {"max_n": 8}, {"category_dimension": "bbox"},
                       {"streams": [{**stream, "count": 1}]},
                       {"streams": [{**stream, "count": "2"}]}):
            with self.subTest(change=change):
                self.write_v3_manifest([stream], 9, **change)
                with self.assertRaises(ValueError):
                    open_index(str(self.root))
        self.write_v3_manifest([stream], 9)
        chunk = self.root / name / "shapes_000001.bin"
        chunk.write_bytes((1 << 87).to_bytes(11, "little"))
        index = open_index(str(self.root))
        with self.assertRaises(ValueError):
            list(index.iter_range(0, 0))
        chunk.write_bytes(b"12345678")
        with self.assertRaises(ValueError):
            open_index(str(self.root))


if __name__ == "__main__":
    unittest.main()
