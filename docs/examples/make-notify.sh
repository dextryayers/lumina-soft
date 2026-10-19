#!/usr/bin/env sh
set -u

make "$@"
status=$?
label="make $*"

if [ "$status" -eq 0 ]; then
    lumina-send notify \
        --title "Build succeeded" \
        --body "$label" \
        --accent "#34c759" \
        --duration-ms 5000 >/dev/null 2>&1 || true
else
    lumina-send notify \
        --title "Build failed ($status)" \
        --body "$label" \
        --accent "#ff3b30" \
        --duration-ms 8000 >/dev/null 2>&1 || true
fi

exit "$status"
