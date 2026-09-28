#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

# Reuse the real Servo-to-Surface QEMU acceptance; M18-B requires its own
# post-present marker in addition to the unchanged M17 first-pixel proof.
sh tests/acceptance/m17_servo_first_web_pixel.sh

serial_log="$repository_root/out/logs/m17-servo.log"
grep -F 'Nagi M18B Albert chrome presented' "$serial_log" >/dev/null
printf '%s\n' 'PASS M18-B Albert chrome was composited and presented on the Nagi Surface'
