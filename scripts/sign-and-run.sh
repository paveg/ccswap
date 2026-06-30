#!/bin/sh
# Cargo runner (macOS dev only): stamp a stable code signature on the freshly
# built binary before running it, so the Keychain "Always Allow" grant survives
# rebuilds. adhoc signatures pin trust to the cdhash, which changes every build;
# signing with the `ccswap-dev` identity gives a cdhash-independent Designated
# Requirement that one "Always Allow" keeps satisfying.
#
# When the dev identity is absent (CI, other contributors), the binary runs
# unsigned exactly as before.
set -e

bin="$1"
shift

if security find-identity -p codesigning 2>/dev/null | grep -q ccswap-dev; then
	codesign --force --sign ccswap-dev --identifier dev.ccswap.cli "$bin" >/dev/null 2>&1 || true
fi

exec "$bin" "$@"
