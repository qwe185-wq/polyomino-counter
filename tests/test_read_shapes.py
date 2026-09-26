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

from read_shapes import main, open_index


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


if __name__ == "__main__":
    unittest.main()
