#!/usr/bin/env bash
# Gives Meetily back its data after Assunta moved it (macOS).
#
# Assunta 0.4.1 (first build after the rename) moved ~/Library/Application Support/com.meetily.ai
# into com.assunta.app on first launch, leaving Meetily empty. This copies Assunta's data folder
# back so Meetily sees its meetings again. Assunta keeps its copy.
#
# Usage: quit Meetily and Assunta, then run
#   ./scripts/restore-meetily-data.sh
set -euo pipefail

SUPPORT="$HOME/Library/Application Support"
SRC="$SUPPORT/com.assunta.app"
DST="$SUPPORT/com.meetily.ai"
DB="meeting_minutes.sqlite"
# Migrations that only exist in Assunta. Meetily refuses to open a database that
# lists migrations it doesn't know, so their records are removed from the copy
# (the extra tables and columns themselves are harmless to Meetily).
ASSUNTA_FIRST_MIGRATION=20260929000000

if pgrep -xiq "meetily|assunta"; then
  echo "Please quit Meetily and Assunta first." >&2
  exit 1
fi
if [ ! -f "$SRC/$DB" ]; then
  echo "No Assunta database found at $SRC/$DB — nothing to restore." >&2
  exit 1
fi
command -v sqlite3 >/dev/null || { echo "sqlite3 is required (it ships with macOS)." >&2; exit 1; }

if [ -e "$DST" ]; then
  BACKUP="$DST.backup-$(date +%Y%m%d-%H%M%S)"
  echo "Keeping the current Meetily folder as: $BACKUP"
  mv "$DST" "$BACKUP"
fi

echo "Copying $SRC → $DST"
mkdir -p "$DST"
cp -R "$SRC/." "$DST/"

echo "Making the database compatible with Meetily"
sqlite3 "$DST/$DB" "PRAGMA wal_checkpoint(TRUNCATE); DELETE FROM _sqlx_migrations WHERE version >= $ASSUNTA_FIRST_MIGRATION;"

echo "Done. Open Meetily: your meetings should be back. Assunta still has its own copy."
