#!/usr/bin/env python3
"""Developer convenience entry point for the standalone native Mac release bundle."""
from pathlib import Path
import argparse
import os
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path,
                        default=ROOT / "target/macos-button-research/release/IrisScope.app")
    parser.add_argument("--check", action="store_true",
                        help="Check the bundle without opening USB or authenticating")
    args = parser.parse_args()
    launcher = args.app / "Contents/MacOS/IrisScope"
    if not launcher.is_file():
        raise SystemExit("Prepare the native release bundle first; see the Mac validation report.")
    if os.geteuid() == 0:
        raise SystemExit("Launch as the normal user. Only the bundled USB helper needs administrator rights.")
    raise SystemExit(subprocess.call([str(launcher), *(["--check-launcher"] if args.check else [])]))


if __name__ == "__main__":
    main()
