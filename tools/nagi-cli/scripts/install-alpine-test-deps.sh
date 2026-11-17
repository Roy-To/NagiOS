#!/bin/sh
set -eu

apk_bin=${APK:-apk}
repository=${NAGI_APK_REPOSITORY:-/tmp/apkrepo/main}
template_root=${NAGI_GIT_TEMPLATE_ROOT:-/usr/share/git-core/templates}
install_log=${NAGI_APK_INSTALL_LOG:-/tmp/nagi-cli-apk-install.log}

if ! command -v "$apk_bin" >/dev/null 2>&1; then
    printf '%s\n' 'APK_TRUSTED_INSTALL=FAIL (apk executable is unavailable)' >&2
    exit 2
fi

if "$apk_bin" info -e git >/dev/null 2>&1 || \
    "$apk_bin" info -e git-init-template >/dev/null 2>&1; then
    printf '%s\n' 'APK_TRUSTED_INSTALL=FAIL (packages were present before offline verification)' >&2
    exit 1
fi

if "$apk_bin" --no-network --repository="$repository" add git git-init-template \
    >"$install_log" 2>&1; then
    install_status=0
else
    install_status=$?
fi
cat "$install_log"
if [ "$install_status" -ne 0 ]; then
    if grep -Eiq 'untrusted|signature' "$install_log"; then
        printf 'APK_SIGNATURE=FAIL apk_exit=%s\n' "$install_status" >&2
    else
        printf 'APK_TRUSTED_INSTALL=FAIL apk_exit=%s\n' "$install_status" >&2
    fi
    exit "$install_status"
fi

if ! "$apk_bin" info -e git >/dev/null 2>&1 || \
    ! "$apk_bin" info -e git-init-template >/dev/null 2>&1; then
    printf '%s\n' 'APK_TRUSTED_INSTALL=FAIL (required packages are not installed)' >&2
    exit 1
fi

printf '%s\n' 'APK_SIGNATURE=PASS (apk default trusted-key path; no untrusted override)'
printf '%s\n' 'APK_TRUSTED_INSTALL=PASS'

if [ ! -d "$template_root" ] || [ -L "$template_root" ] || \
    [ ! -s "$template_root/info/exclude" ] || [ -L "$template_root/info/exclude" ] || \
    [ ! -f "$template_root/description" ] || [ -L "$template_root/description" ]; then
    printf '%s\n' 'GIT_INIT_TEMPLATE=FAIL (required template files are missing or incomplete)' >&2
    exit 1
fi

printf '%s\n' 'GIT_INIT_TEMPLATE=PASS'
printf '%s\n' 'GUEST_GIT_READY=PASS'
