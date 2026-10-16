#!/usr/bin/env python3
"""Verify linker SDK discovery includes frameworks relocated behind symlinks."""
from pathlib import Path
import subprocess
import tempfile
import unittest


class LinkerSDKTests(unittest.TestCase):
    def test_framework_directory_symlinks_are_followed(self):
        source = Path(__file__).with_name("zigbuild-provision-vm.sh").read_text()
        commands = source.split('    cd "$SDK"\n', 1)[1].split(') >"$TBD_LIST"', 1)[0]
        with tempfile.TemporaryDirectory() as temporary:
            sdk = Path(temporary)
            for directory in ["System/Library/Frameworks", "System/Library/PrivateFrameworks", "usr/lib"]:
                (sdk / directory).mkdir(parents=True)
            webkit = sdk / "System/Cryptexes/OS/System/Library/Frameworks/WebKit.framework"
            webkit.mkdir(parents=True)
            (webkit / "WebKit.tbd").write_text("link stub")
            (sdk / "System/Library/Frameworks/WebKit.framework").symlink_to(
                "../../../System/Cryptexes/OS/System/Library/Frameworks/WebKit.framework")
            paths = subprocess.check_output(["bash", "-ec", commands], cwd=sdk, text=True).splitlines()
            self.assertIn("System/Library/Frameworks/WebKit.framework/WebKit.tbd", paths)


if __name__ == "__main__":
    unittest.main()
