#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

# Reuse the real Servo-to-Surface QEMU acceptance; CI sets this when the
# immediately preceding M17 step already produced the serial log.
if [ "${NAGI_M18B_REUSE_M17_ACCEPTANCE:-0}" != "1" ]; then
    sh tests/acceptance/m17_servo_first_web_pixel.sh
fi

serial_log="$repository_root/out/logs/m17-servo.log"
grep -F 'Nagi M18B Albert chrome presented' "$serial_log" >/dev/null
printf '%s\n' 'PASS M18-B Albert chrome was composited and presented on the Nagi Surface'
