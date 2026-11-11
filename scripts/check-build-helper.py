#!/usr/bin/env python3
"""Exercise cleanup boundaries with fake Cargo; never touch the real target/."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).with_name("build.sh")
TAG = "Signature: 8a477f597d28d172789f06886806bc55\n"


class BuildHelper(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="carrel build helper ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        shutil.copy2(SOURCE, self.root / "scripts/build.sh")
        (self.root / "target").mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        cargo = self.bin / "cargo"
        cargo.write_text("""#!/usr/bin/env python3
import json, os, sys
with open(os.environ["CALL_LOG"], "a") as log:
    log.write(json.dumps({"args": sys.argv[1:], "build_dir": os.environ["CARGO_BUILD_BUILD_DIR"]}) + "\\n")
if sys.argv[1] == "clean":
    sys.exit(int(os.environ.get("CLEAN_STATUS", "0")))
""")
        cargo.chmod(0o755)
        du = self.bin / "du"
        du.write_text('#!/bin/sh\nprintf "2048\\ttarget\\n"\n')
        du.chmod(0o755)
        self.log = self.root / "calls"
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        CALL_LOG=str(self.log), CARREL_BUILD_CACHE_MIB="1",
                        CARGO_TARGET_DIR="/other/target", CARGO_BUILD_BUILD_DIR="/other/build")

    def run_helper(self, *args):
        return subprocess.run([str(self.root / "scripts/build.sh"), *args],
                              env=self.env, capture_output=True, text=True, check=False)

    def calls(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_untagged_output_is_built_without_being_cleaned(self):
        result = self.run_helper("--locked")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]["args"][0], "build")
        self.assertIn("Skipping", result.stderr)

    def test_tagged_output_is_cleaned_and_both_directories_are_local(self):
        (self.root / "target/CACHEDIR.TAG").write_text(TAG)
        result = self.run_helper("--release")
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.calls()
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0]["args"][0], "clean")
        for call in calls:
            self.assertEqual(call["args"][1:3], ["--target-dir", str(self.root / "target")])
            self.assertEqual(call["build_dir"], str(self.root / "target"))

    def test_a_symlinked_helper_builds_its_own_checkout(self):
        elsewhere = self.root / "elsewhere"
        (elsewhere / "bin").mkdir(parents=True)
        (elsewhere / "target").mkdir()
        (elsewhere / "target/CACHEDIR.TAG").write_text(TAG)
        (self.root / "target/CACHEDIR.TAG").write_text(TAG)
        alias = elsewhere / "bin/build-carrel"
        alias.symlink_to("../../scripts/build.sh")
        result = subprocess.run([str(alias)], env=self.env,
                                capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 2)
        for call in self.calls():
            self.assertEqual(call["args"][1:3], ["--target-dir", str(self.root / "target")])
            self.assertEqual(call["build_dir"], str(self.root / "target"))

    def test_a_symlink_cannot_make_a_directory_cleanable(self):
        (self.root / "marker").write_text(TAG)
        (self.root / "target/CACHEDIR.TAG").symlink_to(self.root / "marker")
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]["args"][0], "build")

    def test_a_malformed_marker_does_not_allow_cleanup(self):
        (self.root / "target/CACHEDIR.TAG").write_text("not a cache marker")
        result = self.run_helper()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]["args"][0], "build")

    def test_below_the_limit_does_not_clean_even_with_a_valid_marker(self):
        (self.root / "target/CACHEDIR.TAG").write_text(TAG)
        self.env["CARREL_BUILD_CACHE_MIB"] = "2"
        self.assertEqual(self.run_helper().returncode, 0)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]["args"][0], "build")

    def test_a_failed_clean_stops_before_building(self):
        (self.root / "target/CACHEDIR.TAG").write_text(TAG)
        self.env["CLEAN_STATUS"] = "1"
        self.assertEqual(self.run_helper().returncode, 1)
        self.assertEqual(len(self.calls()), 1)
        self.assertEqual(self.calls()[0]["args"][0], "clean")

    def test_invalid_limits_do_not_touch_cargo(self):
        for limit in ["0", "-1", "1.5", "08", "10000000", "hello"]:
            with self.subTest(limit=limit):
                self.env["CARREL_BUILD_CACHE_MIB"] = limit
                self.assertEqual(self.run_helper().returncode, 2)
                self.assertEqual(self.calls(), [])

    def test_shared_target_and_overrides_are_refused(self):
        for args in [("--target-dir", "/other"), ("--manifest-path=x",),
                     ("-m", "/other/Cargo.toml"), ("--config", 'build.build-dir="/other"')]:
            with self.subTest(args=args):
                result = self.run_helper(*args)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(self.calls(), [])
        (self.root / "target").rmdir()
        (self.root / "shared").mkdir()
        (self.root / "target").symlink_to(self.root / "shared")
        self.assertEqual(self.run_helper().returncode, 2)
        self.assertEqual(self.calls(), [])


if __name__ == "__main__":
    unittest.main()
