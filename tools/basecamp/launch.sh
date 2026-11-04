#!/usr/bin/env bash
# launch.sh — start the Basecamp instance with the OSM app.
# Relaunch the app + module after this (Module/Apps Inspector Load) if needed.
set -u
export DISPLAY=:99
export QT_QUICK_BACKEND=software
export LIBGL_ALWAYS_SOFTWARE=1
export LD_LIBRARY_PATH=/home/codespace/.local/share/Logos/LogosBasecamp/modules/osm
cd /tmp/basecamp
exec ./LogosBasecamp-x86_64.AppImage \
  "$@" > /tmp/basecamp/basecamp.log 2>&1
