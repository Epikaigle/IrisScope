#!/bin/zsh
set -eu
task_root=$(cd "$(dirname "$0")/../.." && pwd)
exec python3 "$task_root/scripts/macos/launch-usb-experiment.py"
