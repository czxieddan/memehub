import hashlib
import tomllib
import unittest

from support import PNG, entry, manifests
from memehub.protocol import (Duplicate, Invalid, MAX_ENTRIES, MAX_IMAGE, MAX_INDEX, check_image,
                              index_bytes, manifest_bytes, read_entries,
                              parse_toml, unique_entries, validate_manifests)


def entries(files):
    return read_entries(files, lambda path, limit: files[path])


class ProtocolTests(unittest.TestCase):
    def test_exact_manifest_fields_and_raw_byte_hash(self):
        item = entries(entry())["7/Pig"]
        fields = tomllib.loads(manifest_bytes(item).decode())
        self.assertEqual(fields, {"schema_version": 1, "hash": hashlib.sha256(PNG).hexdigest(),
                                  "uploader_id": 7, "name": "Pig", "display_name": "大猪猪",
                                  "path": "/data/7/Pig/meme.png"})
        self.assertIs(type(fields["uploader_id"]), int)

    def test_unicode_metadata_does_not_change_image_hash(self):
        first = entries(entry(display="猪猪 🐷"))["7/Pig"]
        second = entries(entry(display="另一名字"))["7/Pig"]
        self.assertEqual(first.digest, second.digest)
        self.assertNotEqual(manifest_bytes(first), manifest_bytes(second))
        self.assertEqual(tomllib.loads(manifest_bytes(first).decode())["display_name"], "猪猪 🐷")

    def test_toml_escaping_round_trip(self):
        files = entry()
        files["data/7/Pig/meme.toml"] = b'file="meme.png"\ndisplay_name="a\\n\\\"b\\\\c"\n'
        item = entries(files)["7/Pig"]
        self.assertEqual(tomllib.loads(manifest_bytes(item).decode())["display_name"], 'a\n"b\\c')

    def test_index_fields_sorted_empty_and_source_commit(self):
        fields = tomllib.loads(index_bytes("a" * 40, ["f" * 64, "0" * 64]).decode())
        self.assertEqual(fields, {"schema_version": 1, "source_commit": "a" * 40,
                                  "hashes": ["0" * 64, "f" * 64]})
        self.assertEqual(tomllib.loads(index_bytes("a" * 40, []).decode())["hashes"], [])
        with self.assertRaises(Invalid):
            index_bytes("main", [])

    def test_maximum_entry_index_fits_its_own_read_limit(self):
        hashes = [f"{number:064x}" for number in range(MAX_ENTRIES)]
        content = index_bytes("a" * 40, hashes)
        self.assertGreater(len(content), 32 * 1024)
        self.assertLessEqual(len(content), MAX_INDEX)
        self.assertEqual(parse_toml(content, "manifest/index.toml", MAX_INDEX)["hashes"], hashes)

    def test_control_character_metadata_still_generates_valid_toml(self):
        files = entry()
        files["data/7/Pig/meme.toml"] = b'file="meme.png"\ndisplay_name="a\\u007fb"\n'
        item = entries(files)["7/Pig"]
        self.assertEqual(tomllib.loads(manifest_bytes(item).decode())["display_name"], "a\x7fb")

    def test_all_manifests_must_match_bytes_and_metadata(self):
        files = manifests(entry())
        validate_manifests(files, lambda p, limit: files[p], entries(files))
        path = next(path for path in files if path.startswith("manifest/"))
        for replacement in (files[path].replace(b"schema_version = 1", b"schema_version = true"),
                            files[path].replace("大猪猪".encode(), "伪造".encode()),
                            files[path] + b'extra = "bad"\n'):
            with self.subTest(replacement=replacement):
                broken = {**files, path: replacement}
                with self.assertRaises(Invalid):
                    validate_manifests(broken, lambda p, limit: broken[p], entries(broken))

    def test_missing_extra_and_stale_manifests_fail(self):
        files = entry()
        with self.assertRaises(Invalid):
            validate_manifests(files, lambda p, limit: files[p], entries(files))
        files = manifests(files)
        files["manifest/" + "0" * 64 + "/manifest.toml"] = b""
        with self.assertRaises(Invalid):
            validate_manifests(files, lambda p, limit: files[p], entries(files))

    def test_same_user_different_name_is_duplicate(self):
        with self.assertRaises(Duplicate) as caught:
            unique_entries(entries({**entry(), **entry(name="Other")}))
        self.assertIn(hashlib.sha256(PNG).hexdigest(), str(caught.exception))
        self.assertIn("/data/7/Other/meme.png", str(caught.exception))

    def test_names_paths_and_exactly_one_image(self):
        for name in ("中文", "a" * 65, "../escape", "a.b", "a b"):
            with self.subTest(name=name), self.assertRaises(Invalid):
                entries(entry(name=name))
        for addition in ("data/7/Pig/second.png", "data/7/Pig/nested/meme.png"):
            with self.subTest(addition=addition), self.assertRaises(Invalid):
                entries({**entry(), addition: PNG})
        files = entry()
        files["data/7/Pig/meme.toml"] = b'file="../escape.png"\ndisplay_name="x"'
        with self.assertRaises(Invalid):
            entries(files)

    def test_image_limits_extensions_and_signatures(self):
        for data, path in ((b"", "a.png"), (b"not png", "a.png"), (PNG, "a.gif"),
                           (PNG, "a.svg"), (PNG + b"x" * MAX_IMAGE, "a.png")):
            with self.subTest(path=path), self.assertRaises(Invalid):
                check_image(data, path)
        for data, extension in ((PNG, "png"), (b"\xff\xd8\xffx", "jpeg"),
                                (b"GIF89ax", "gif"), (b"RIFF0000WEBPx", "webp")):
            self.assertEqual(check_image(data, "x." + extension), hashlib.sha256(data).hexdigest())


if __name__ == "__main__":
    unittest.main()
