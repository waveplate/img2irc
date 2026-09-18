import json
import os
import platform
import subprocess
import sys
import sysconfig
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(
    sys.platform == "linux" and platform.machine() == "x86_64",
    "the wheel builder currently targets x86_64 Linux",
)
class WheelBuildTests(unittest.TestCase):
    def docker_arguments(self, *options):
        # Exercise the public script without starting a container or downloading
        # toolchains. In particular, paths with spaces must stay single args.
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            docker = parent / "docker"
            docker.write_text(
                "#!/usr/bin/env python3\n"
                "import json, sys\n"
                "print(json.dumps(sys.argv[1:]))\n"
            )
            docker.chmod(0o755)
            output = parent / "wheel output"
            result = subprocess.run(
                [str(ROOT / "scripts/build-wheel.sh"), "--python", sys.executable,
                 "--out", str(output), *options],
                cwd=parent,
                env={**os.environ, "PATH": str(parent) + os.pathsep + os.environ["PATH"],
                     "IMG2IRC_BUILD_JOBS": "2"},
                text=True, capture_output=True, check=True,
            )
            calls = [json.loads(line) for line in result.stdout.splitlines()
                     if line.startswith("[")]
            build, arguments = calls
            self.assertEqual(build[0], "build")
            self.assertIn(
                "MANYLINUX_IMAGE=" + os.environ.get(
                    "IMG2IRC_MANYLINUX_IMAGE", "quay.io/pypa/manylinux_2_28_x86_64"
                ), build,
            )
            self.assertIn(f"type=bind,source={output},target=/wheelhouse", arguments)
            self.assertIn("IMG2IRC_BUILD_JOBS=2", arguments)
            self.assertIn("--user", arguments)
            self.assertIn("IMG2IRC_MANYLINUX_IN_CONTAINER=1", arguments)
            return arguments

    def test_default_build_selects_manylinux_and_matching_python_with_ocr(self):
        arguments = self.docker_arguments()
        self.assertIn("img2irc-manylinux_2_28-builder", arguments)
        index = arguments.index("/io/scripts/build-manylinux-wheel.sh")
        version = f"cp{sys.version_info.major}{sys.version_info.minor}"
        suffix = "t" if sysconfig.get_config_var("Py_GIL_DISABLED") else ""
        self.assertEqual(arguments[index + 1:], [version + "-" + version + suffix, "1"])

    def test_no_ocr_and_extra_arguments_are_forwarded(self):
        arguments = self.docker_arguments("--no-ocr", "--", "--offline")
        self.assertEqual(arguments[-2:], ["0", "--offline"])

    def test_container_helper_refuses_to_build_on_host(self):
        environment = dict(os.environ)
        environment.pop("IMG2IRC_MANYLINUX_IN_CONTAINER", None)
        result = subprocess.run(
            ["bash", str(ROOT / "scripts/build-manylinux-wheel.sh")],
            env=environment, text=True, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("run scripts/build-wheel.sh", result.stderr)


if __name__ == "__main__":
    unittest.main()
