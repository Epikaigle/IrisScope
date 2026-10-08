#!/usr/bin/env python3
"""Render deterministic Linux UI fixtures at four scale factors and compare baselines.

Run under Xvfb with a 4096x2304 screen. --update is an explicit developer action;
CI only compares reviewed baselines and uploads new captures on failure.
"""
import argparse
import os
from pathlib import Path
import subprocess

from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[1]
BASELINES = ROOT / "tests" / "visual-baselines"
SCENES = {
    "viewer-tools-menu", "viewer-view-menu", "library-sparse",
    "camera-angle-popup", "viewer-angle-popup", "references-loaded", "references-empty",
    "camera-rotation", "camera-presentation-idle", "viewer-rotation", "library-menu", "dossier-empty",
    "camera-after-capture", "camera-narrow-after-capture", "camera-mirror-popup", "camera-click-selection",
    "camera-active", "camera-controls", "patient-search", "settings",
    "camera-fullscreen", "camera-fullscreen-photo", "camera-fullscreen-blocked",
    "camera-fullscreen-hidden", "viewer-video", "viewer-comparison", "patient-edit", "library-query", "capture-notice",
    "settings-bottom", "settings-backup", "settings-reminder", "viewer-export",
    "dossier-session", "calendar-filter", "viewer-photo-export",
    "camera-presentation", "presentation-notes-error", "camera-resized", "viewer-presentation", "viewer-image-only", "viewer-loading-notes", "viewer-resized", "library-small", "library-large", "library-list-large", "settings-advanced",
    "viewer-photo", "viewer-notes", "viewer-references", "viewer-display", "viewer-chooser", "viewer-zoom", "viewer-loupe", "library-custom", "library-minimum",
}
SIZES = {"800x600", "1360x860"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update", action="store_true", help="Replace reviewed visual baselines")
    parser.add_argument("--compare-only", action="store_true", help="Compare existing captures without rebuilding or rendering")
    parser.add_argument("--release", action="store_true", help="Render with optimized fixtures to accelerate large matrices")
    parser.add_argument("--scenes", help="Render and compare only these comma-separated baseline scenes")
    parser.add_argument("--output", type=Path, default=ROOT / "target" / "ui-visuals")
    args = parser.parse_args()
    args.output = args.output.resolve()
    if args.update and args.compare_only:
        parser.error("--update requires fresh captures; omit --compare-only")
    if not args.compare_only:
        command = ["cargo", "build", "-p", "iriscope-app", "--example", "ui_snapshots", "--locked"]
        if args.release:
            command.append("--release")
        subprocess.run(command, cwd=ROOT, check=True)
    executable = ROOT / "target" / ("release" if args.release else "debug") / "examples" / "ui_snapshots"
    scenes = SCENES if not args.scenes else set(args.scenes.split(","))
    if not scenes or not scenes <= SCENES:
        parser.error("--scenes must name existing baseline scenes")
    failures = []
    checked = 0
    updated = 0
    for scale, label in [("1", "100"), ("1.25", "125"), ("1.5", "150"), ("2", "200")]:
        destination = args.output / f"scale-{label}"
        destination.mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, SLINT_BACKEND="winit-software", SLINT_STYLE="fluent", SLINT_SCALE_FACTOR=scale)
        env.pop("IRISCOPE_SNAPSHOT_SCENES", None)
        env.pop("IRISCOPE_SNAPSHOT_IMAGE", None)
        if args.scenes:
            env["IRISCOPE_SNAPSHOT_SCENES"] = args.scenes
        if not args.compare_only:
            subprocess.run([str(executable), str(destination)], cwd=ROOT, env=env, check=True)
        for scene in sorted(scenes):
            for size in sorted(SIZES):
                for theme in ["light", "dark"]:
                    filename = f"{scene}-{size}-{theme}.png"
                    actual = destination / filename
                    reference = BASELINES / f"scale-{label}" / filename
                    if args.update:
                        reference.parent.mkdir(parents=True, exist_ok=True)
                        with Image.open(actual) as capture:
                            capture.convert("RGB").save(reference, optimize=True, compress_level=9)
                        updated += 1
                        continue
                    if not reference.exists():
                        failures.append(f"Missing baseline: {reference.relative_to(ROOT)}")
                        continue
                    if not actual.exists():
                        failures.append(f"Missing capture: {actual}")
                        continue
                    with Image.open(reference) as before, Image.open(actual) as after:
                        if before.size != after.size:
                            failures.append(f"Dimensions changed: scale-{label}/{filename}")
                            continue
                        difference = ImageChops.difference(before.convert("RGB"), after.convert("RGB"))
                        # Tiny rasterization differences are tolerated, while one
                        # misplaced separator across the toolbar exceeds the budget.
                        red, green, blue = difference.split()
                        maximum = ImageChops.lighter(ImageChops.lighter(red, green), blue)
                        changed = sum(maximum.histogram()[13:])
                        fraction = changed / (before.width * before.height)
                        if fraction > 0.0005:
                            difference.save(destination / f"diff-{filename}")
                            failures.append(f"Visual change ({fraction:.2%}): scale-{label}/{filename}")
                        checked += 1
    if failures:
        print("\n".join(failures))
        raise SystemExit(1)
    if args.update:
        print(f"Updated {updated} visual baselines.")
    elif args.compare_only:
        print(f"Passed {checked} image comparisons using existing captures.")
    else:
        rendered = sum(
            sum(not path.name.startswith("diff-") for path in (args.output / f"scale-{label}").glob("*.png"))
            for label in ["100", "125", "150", "200"]
        )
        print(f"Passed {checked} image comparisons; captures available: {rendered} at 100%, 125%, 150%, 200%.")


if __name__ == "__main__":
    main()
