#!/usr/bin/env bash
# Install a root-owned, read-only USB status bridge. IrisScope stays unprivileged.
set -euo pipefail
if [[ "$EUID" -ne 0 ]]; then
    echo "Administrator authentication is required to install the DE400 button bridge." >&2
    exit 1
fi
project_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
install -d -o root -g root -m 0755 /usr/local/lib/iriscope
install -o root -g root -m 0644 "$project_root/helpers/linux/de400_button_bridge.py" /usr/local/lib/iriscope/de400_button_bridge.py
install -o root -g root -m 0644 "$project_root/helpers/linux/iriscope-button.service" /etc/systemd/system/iriscope-button.service
install -d -o root -g root -m 0755 /etc/modules-load.d
printf '%s\n' usbmon > /etc/modules-load.d/iriscope-button.conf
chmod 0644 /etc/modules-load.d/iriscope-button.conf
/usr/sbin/modprobe usbmon
systemctl daemon-reload
systemctl enable iriscope-button.service
systemctl restart iriscope-button.service
systemctl --no-pager --full status iriscope-button.service
