#!/usr/bin/env bash
# record-demo.sh — record scripts/demo.sh as an asciinema cast, detached.
#
# Produces docs/demo-evidence/osm-demo-dev0-<stamp>.cast. The cast is plain
# text (input + output events); the user replays it and records the voiceover
# on top later (asciinema play + any screen recorder), or converts it with agg.
#
# Design notes:
# - Wide terminal (200x50): "zoomed out" — more columns and scrollback lines
#   visible per frame; no overlays or annotations are added, only the demo's
#   own output. The size is set by the tmux pane (-x 200 -y 50) BEFORE
#   asciinema allocates its recording pty, so the cast header and the captured
#   content are both genuinely 200x50 (a post-hoc `stty` inside the recorded
#   command would be too late — asciinema would still record at 80x24).
# - --idle-time-limit 2: collapses silent proof-generation pauses on replay,
#   so the final narrated video is minutes, not the raw wall-clock ~20 min.
# - Launched inside a detached tmux session: survives the agent session
#   entirely; this script is fire-and-forget from the caller's perspective.
#   The session self-destructs when the recording finishes.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUT_DIR="$REPO_DIR/docs/demo-evidence"
mkdir -p "$OUT_DIR"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
CAST="$OUT_DIR/osm-demo-dev0-$STAMP.cast"
LOG="$OUT_DIR/osm-demo-dev0-$STAMP.launch.log"
STATUS="$OUT_DIR/osm-demo-dev0-$STAMP.status"
SESSION="osm-demo-rec-$STAMP"

# Inner runner: asciinema rec -> write status -> kill the tmux session.
# Written to a temp file so tmux's sh -c doesn't need fragile nested quoting.
INNER=/tmp/osm-demo-runner-$STAMP.sh
cat > "$INNER" <<EOF
#!/usr/bin/env bash
set -uo pipefail
CAST='$CAST'
STATUS='$STATUS'
LOG='$LOG'
REPO_DIR='$REPO_DIR'
SESSION='$SESSION'
asciinema rec \\
  --overwrite \\
  --idle-time-limit 2 \\
  --title 'LP-0018 OSM end-to-end demo (RISC0_DEV_MODE=0)' \\
  -c "\$REPO_DIR/scripts/demo.sh" \\
  "\$CAST" > "\$LOG" 2>&1
ec=\$?
echo "finished: \$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "\$STATUS"
echo "exit:     \$ec" >> "\$STATUS"
echo "cast:     \$CAST (\$(wc -c < "\$CAST") bytes)" >> "\$STATUS"
tmux kill-session -t "\$SESSION" 2>/dev/null || true
EOF
chmod +x "$INNER"

echo "recording: $CAST" | tee "$STATUS"
echo "started:  $(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$STATUS"
echo "session:  $SESSION (tmux, 200x50)" >> "$STATUS"

# Detached tmux session at 200x50. asciinema inherits the pane size.
tmux new-session -d -s "$SESSION" -x 200 -y 50 "bash $INNER"

echo "launched tmux session: $SESSION"
echo "watch:  tmux attach -t $SESSION"
echo "status: cat $STATUS"
