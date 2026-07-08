#!/bin/sh
set -eu

APP=/app/livrarr

fatal() { echo "FATAL: $*" >&2; exit 1; }

# --- Argument handling ---
# Default the app invocation only when NO command was given. If the first arg is a
# flag, run the app with defaults + those flags. Otherwise run the given command as-is
# (debug escape hatch, e.g. `docker run … sh`). Never silently discard user args.
if [ "$#" -eq 0 ]; then
    set -- "$APP" --data /config --ui-dir /app/ui
elif [ "${1#-}" != "$1" ]; then
    set -- "$APP" --data /config --ui-dir /app/ui "$@"
fi

# --- Validate PUID/PGID up front (mode-independent) ---
# Invalid values are rejected even in the already-non-root path (where they are otherwise
# ignored), so the "reject non-zero / no leading zeros" rule holds uniformly. Empty
# (e.g. `PUID=${PUID}` with an unset .env) defaults to 1000 (friendly + non-root).
PUID="${PUID:-1000}"
PGID="${PGID:-1000}"
# Reject non-numeric AND any leading-zero form: su-exec/find read "00" as uid 0, so a
# string-exact "0" check would be insufficient — any leading zero is rejected.
case "$PUID" in *[!0-9]*|0*) fatal "PUID must be a non-zero numeric uid without leading zeros (got '$PUID')";; esac
case "$PGID" in *[!0-9]*|0*) fatal "PGID must be a non-zero numeric gid without leading zeros (got '$PGID')";; esac

# --- Already non-root (user:/--user, or a rootless engine): nothing privileged to do ---
if [ "$(id -u)" != "0" ]; then
    exec /sbin/tini -- "$@"
fi

# --- Running as root → PUID/PGID mode ---
# Repair ONLY the mis-owned entries under /config (self-healing across an interrupted prior
# boot). -h keeps a symlink inside /config from redirecting the chown outside it.
if ! find /config \( ! -user "$PUID" -o ! -group "$PGID" \) -exec chown -h "$PUID:$PGID" {} +; then
    fatal "cannot chown /config — PUID/PGID mode needs the CHOWN (+DAC_OVERRIDE) capability.
  Fix: add cap_add:[CHOWN,SETUID,SETGID,DAC_OVERRIDE], OR run with user:\"$PUID:$PGID\" and pre-chown ./config on the host."
fi

# Preflight the privilege drop so a missing SETUID/SETGID fails CLEARLY even when no chown
# was needed (otherwise su-exec would emit only a cryptic error).
if ! su-exec "$PUID:$PGID" true 2>/dev/null; then
    fatal "cannot drop privileges to $PUID:$PGID — PUID/PGID mode needs the SETUID+SETGID capabilities.
  Fix: add cap_add:[CHOWN,SETUID,SETGID,DAC_OVERRIDE], OR run with user:\"$PUID:$PGID\"."
fi

# Drop privileges and hand off. Nothing stays root; tini becomes a non-root PID 1 for the app.
exec su-exec "$PUID:$PGID" /sbin/tini -- "$@"
