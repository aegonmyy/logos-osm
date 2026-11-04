#!/usr/bin/env bash
# reload-osm.sh — reload the osm_app view after editing its Main.qml.
# Kills the ui-host, then cycles Apps Inspector Unload->Load (row located by
# OCR — row order shifts between sessions), returns to the app view.
# Prints RELOADED or FAILED. Run after every QML edit.
set -u
UI=/tmp/basecamp/ui.sh
SHOT=/tmp/basecamp

# 1. Kill the app's ui-host. Script file: pgrep -f cannot self-match here.
PID=$(pgrep -f 'ui-host --name osm_app' | head -1)
[ -n "$PID" ] && kill -9 "$PID" 2>/dev/null
sleep 1

# 2. Navigate: rail Settings icon -> Apps Inspector section.
$UI click 43 710;  sleep 1.5
$UI click 230 212; sleep 1.5

# 3. Locate the osm_app row by OCR (row order changes between sessions).
cd "$SHOT"
DISPLAY=:99 import -window root -depth 8 ui_full.png
ROW_Y=$(tesseract ui_full.png stdout tsv 2>/dev/null | awk -F'\t' '$12 ~ /osm_app/ {print $8; exit}')
if [ -z "$ROW_Y" ]; then
  echo "FAILED: osm_app row not visible in Apps Inspector (scroll or re-navigate)"
  exit 1
fi
BTN_Y=$((ROW_Y + 12))
$UI click 1165 $BTN_Y; sleep 2.5    # Unload (row may say Unload even if dead)
$UI click 1165 $BTN_Y; sleep 4      # Load
$UI click 43 92;    sleep 1.5        # rail Workspace icon: app view forward

if pgrep -f 'ui-host --name osm_app' >/dev/null; then
  echo "RELOADED: ui-host alive"
else
  echo "FAILED: no ui-host — QML has a syntax or import error (revert last edit)"
fi
