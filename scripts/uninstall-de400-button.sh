#!/usr/bin/env bash
set -euo pipefail
if [[ "$EUID" -ne 0 ]]; then
    echo "Administrator authentication is required to remove the DE400 button bridge." >&2
    exit 1
fi
systemctl disable --now iriscope-button.service
rm -f -- /etc/systemd/system/iriscope-button.service /etc/modules-load.d/iriscope-button.conf /usr/local/lib/iriscope/de400_button_bridge.py
systemctl daemon-reload
rmdir /usr/local/lib/iriscope 2>/dev/null || true
echo "DE400 button bridge removed. Captures and settings were preserved."
