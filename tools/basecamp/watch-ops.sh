#!/usr/bin/env bash
# watch-ops.sh — emit readable op calls + module replies from the wire log.
# The wire tap writes QtRO frames (UTF-16 payload); strip NULs, print heads.
MPID=$(ps -eo pid,args | grep '[l]ogos_host --name osm' | awk '{print $1}' | head -1)
tail -n0 -f /tmp/basecamp/wire.log | while IFS= read -r line; do
  if [[ "$line" == "=== pid $MPID "* ]]; then
    IFS= read -r payload || continue
    clean=$(printf '%s' "$payload" | tr -d '\000')
    printf 'RESULT: %s\n' "${clean:0:230}"
  elif [[ "$line" == "=== pid "* ]]; then
    IFS= read -r payload || continue
    clean=$(printf '%s' "$payload" | tr -d '\000')
    if [[ "$clean" == *invokeOpJson* || "$clean" == *osmVersionJson* ]]; then
      printf 'CALL: %s\n' "${clean:0:160}"
    fi
  fi
done
