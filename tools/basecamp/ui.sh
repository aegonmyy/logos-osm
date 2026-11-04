#!/usr/bin/env bash
# ui.sh — blind-driving helper: screenshot the Xvfb root window, OCR it,
# print words with pixel coordinates. Usage:
#   ui.sh            -> plain OCR text
#   ui.sh tsv        -> word bounding boxes (x y w h conf text)
#   ui.sh click X Y  -> click at coordinates
#   ui.sh type TEXT  -> type a string
#   ui.sh key KEY    -> press a key (Return, Tab, Escape...)
set -u
DISP=":99"
case "${1:-}" in
  tsv)
    DISPLAY=$DISP import -window root -depth 8 /tmp/basecamp/ui.png 2>/dev/null
    tesseract /tmp/basecamp/ui.png stdout tsv 2>/dev/null | awk -F'\t' '$11+0 > 40 && $12 != "" {printf "%5d %5d %4dx%-4d conf=%s  %s\n", $7, $8, $9, $10, $11, $12}'
    ;;
  click)
    DISPLAY=$DISP xdotool mousemove --sync "$2" "$3" click 1
    ;;
  type)
    DISPLAY=$DISP xdotool type -- "$2"
    ;;
  key)
    DISPLAY=$DISP xdotool key "$2"
    ;;
  *)
    DISPLAY=$DISP import -window root -depth 8 /tmp/basecamp/ui.png 2>/dev/null
    tesseract /tmp/basecamp/ui.png stdout 2>/dev/null | grep -v '^[[:space:]]*$'
    ;;
esac
