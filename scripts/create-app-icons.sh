#!/bin/sh
# Developer-only conversion; release packaging uses the checked-in icon files.
set -eu
task_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
task_icons="$task_root/assets/icons"
task_work=$(mktemp -d)
trap 'rm -rf "$task_work"' EXIT HUP INT TERM
magick -background none "$task_icons/iriscope.svg" -resize 1024x1024 "PNG32:$task_icons/iriscope-1024.png"
magick "$task_icons/iriscope-1024.png" -resize 256x256 "PNG32:$task_icons/iriscope-256.png"
magick "$task_icons/iriscope-1024.png" -define icon:auto-resize=256,128,64,48,32,16 "$task_icons/IrisScope.ico"
mkdir "$task_work/IrisScope.iconset"
for task_size in 16 32 128 256 512; do
    magick "$task_icons/iriscope-1024.png" -resize "${task_size}x${task_size}" "PNG32:$task_work/IrisScope.iconset/icon_${task_size}x${task_size}.png"
    task_retina=$((task_size * 2))
    magick "$task_icons/iriscope-1024.png" -resize "${task_retina}x${task_retina}" "PNG32:$task_work/IrisScope.iconset/icon_${task_size}x${task_size}@2x.png"
done
iconutil -c icns "$task_work/IrisScope.iconset" -o "$task_icons/IrisScope.icns"
