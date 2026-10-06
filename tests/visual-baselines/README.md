The PNGs are deterministic synthetic UI fixtures, without camera images or user data.
The 608 references cover the camera toolbar, camera menus, long patient names,
settings, fullscreen commands and capture notices (photo, recording, blocked and hidden),
including long filenames with vertically centered notice actions,
patient editing, library queries, photo comparison, backup progress and reminders,
MP4 export progress, photo tools, presentation privacy, image-only controls,
resizable panels, thumbnail sizes and the video viewer in both themes,
at 800×600 and 1360×860 logical pixels, with 100%, 125%, 150%,
200% scaling.

Linux CI compares them using the Slint software renderer and the Fluent widget style.
It renders 1,536 fixtures including library, viewer and references at four window sizes. The
software checks do not replace testing a camera and native desktop scaling on
Windows and macOS.

Run `xvfb-run -a -s '-screen 0 4096x2304x24 -nolisten tcp' python3 scripts/check-ui-visuals.py --release`.
Pillow is required. For an intentional UI change, add `--update`, inspect the new
captures, and review the PNG changes before committing. CI never updates baselines.
