#!/usr/bin/env bash
#
# package-basecamp.sh — produce standalone, side-loadable `.lgx` bundles for
# the OSM core module and the Basecamp app, plus downloadable assets.
#
# Output: dist/  with  osm.lgx and osm-app.lgx.
#
# Run after `nix build .#osm-lgx` and `.#osm-app-lgx` (or this script runs
# them). The `.lgx` is the artifact an evaluator side-loads into Logos Core.

set -euo pipefail
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
DIST="$REPO_DIR/dist"
mkdir -p "$DIST"

echo "Building osm core .lgx ..."
nix build "$REPO_DIR#osm-lgx" --out-link "$DIST/osm.lgx"
echo "Building osm app .lgx ..."
nix build "$REPO_DIR#osm-app-lgx" --out-link "$DIST/osm-app.lgx"

echo
echo "Bundles in $DIST:"
ls -la "$DIST"/*.lgx
