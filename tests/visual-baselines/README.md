The PNGs are deterministic synthetic UI fixtures, without camera images or user data.
The 928 references cover the camera toolbar, camera menus, long patient names,
settings, fullscreen commands and capture notices (photo, recording, blocked and hidden),
including long filenames with vertically centered notice actions,
patient editing, library queries, photo comparison, backup progress and reminders,
MP4 export progress, photo tools, presentation privacy, image-only controls,
resizable panels, thumbnail sizes and the video viewer in both themes,
including the post-capture sidebar, a narrow sidebar, the mirror popup and
mouse-selected buttons without a keyboard focus outline,
fixed patient fields, compact viewer action menus, theme-consistent overlays,
and a left-aligned sparse library after resizing thumbnails,
the centered dossier action and the final row at maximum thumbnail size,
at 800×600 and 1360×860 logical pixels, with 100%, 125%, 150%,
200% scaling.

Linux CI compares them using the Slint software renderer and the Fluent widget style.
The example offers 66 scenarios including library, viewer and references at four window sizes. The
software checks do not replace testing a camera and native desktop scaling on
Windows and macOS.

Run `xvfb-run -a -s '-screen 0 4096x2304x24 -nolisten tcp' python3 scripts/check-ui-visuals.py --release`.
Pillow is required. For an intentional UI change, add `--update`, inspect the new
captures, and review the PNG changes before committing. CI never updates baselines.

On macOS, set `IRISCOPE_SNAPSHOT_OFFSCREEN=1` to use the same software renderer
without a native event loop or monitor-size limits. `IRISCOPE_SNAPSHOT_SIZES`
can restrict rendering to comma-separated sizes, for example `800x600,1360x860`.
The resulting captures can be checked with `--compare-only` without rebuilding.
