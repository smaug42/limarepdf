#!/bin/bash
# The user guide's screenshots (docs/images), taken from the release app on a virtual X display.
#
#   tools/docshots/docshots.sh OUT [NAME...]          the pictures NAME (default: all) into OUT/NAME.png
#   tools/docshots/docshots.sh OUT try FILE STEP...   one run, to find click points: OUT/try.png
#
# Build first (`cargo build --release`); B and C name other app and command-line binaries. Needs Xvfb
# (xvfb-run), xdotool and ImageMagick 7. The made-up report (mkreport.py) goes into OUT/fix. Each run starts
# its own display, and the app with settings and recovery folders of its own (OUT/cfg-*, removed after):
# the light theme, LimarePDF's teal, "Pat Example" as the name for comments, and Sam Lee's typed
# signatures and initials saved. Only the Digital signatures picture has digital IDs (made-up ones, in
# OUT/signed-cfg). The window is 1000 x 660 points at scale 2 (2000 x 1320 pixels); the guide shows
# each picture at half its width, so it displays at scale 1's size.
#
# Look at each picture before copying it into docs/images: when a control moves, so does its click point
# (find the new one with `try` and s: steps). The dates the app writes are the computer's.
#
# Steps (window pixels; the window is at 0,0):
#   k:KEYS     keys, as xdotool names them     t:TEXT     type TEXT
#   c:X,Y      click                           m:MOD,X,Y  click with MOD held (shift, ctrl)
#   d:X1,Y1,X2,Y2  drag                        h:X,Y      move the pointer only
#   w:X,Y,N    wheel down N notches            u:X,Y,N    wheel up N notches
#   p:SECONDS  wait                            s:NAME     a picture now, OUT/try-NAME.png (try only)
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
B=${B:-$ROOT/target/release/limarepdf}
C=${C:-$ROOT/target/release/limarepdf-cli}
[ $# -ge 1 ] || { sed -n 3,4p "$0" | cut -c3-; exit 2; }
[ -x "$B" ] || { echo "docshots.sh: no app at $B (build it, or set B)" >&2; exit 1; }
[ -x "$C" ] || { echo "docshots.sh: no command-line app at $C (build it, or set C)" >&2; exit 1; }
mkdir -p "$1"
OUT=$(cd "$1" && pwd)
shift
W=2000 H=1320 SCREEN=2400x1600

# ---- Fixtures (all made up) ----
mkdir -p "$OUT/fix" "$OUT/raw" "$OUT/cfg/limarepdf"
python3 -I "$HERE/mkreport.py" "$OUT/fix/field-report.pdf"
python3 -I "$HERE/mkreport.py" "$OUT/fix/plain.pdf" --plain
python3 -I "$HERE/mkreport.py" "$OUT/fix/review.pdf" --review
printf 'theme = light\naccent_colour = limarepdf\ncomment_name = Pat Example\n' > "$OUT/cfg/limarepdf/settings.txt"
printf 'signature = type 1 Sam Lee\nsignature = type 4 Sam Lee\ninitials = type 1 SL\n' \
  > "$OUT/cfg/limarepdf/signatures.txt"

# signed: the plain report signed on page 3 by Pat Example and then Sam Lee, with made-up digital IDs
# (password made-up-pass-1) that OUT/signed-cfg's settings trust, as OUT/fix/field-report-signed.pdf.
signed() {
  local cfg=$OUT/signed-cfg
  rm -rf "$cfg" "$OUT/fix/signed-1.pdf" "$OUT/fix/field-report-signed.pdf"
  cp -r "$OUT/cfg" "$cfg"
  printf 'made-up-pass-1\n' > "$OUT/fix/pw"
  (
    export XDG_CONFIG_HOME=$cfg
    for who in "Pat Example" "Sam Lee"; do
      "$C" id create --name "$who" --organisation "Northern Water Survey" --password-file "$OUT/fix/pw"
    done
    "$C" sign "$OUT/fix/plain.pdf" -o "$OUT/fix/signed-1.pdf" --id "Pat Example" --page 3 \
      --rect 72,680,200,46 --reason Reviewed --password-file "$OUT/fix/pw"
    "$C" sign "$OUT/fix/signed-1.pdf" -o "$OUT/fix/field-report-signed.pdf" --id "Sam Lee" --page 3 \
      --rect 340,680,200,46 --reason Approved --style 2 --password-file "$OUT/fix/pw"
    "$C" trust add "$OUT/fix/field-report-signed.pdf" --signature 1
    "$C" trust add "$OUT/fix/field-report-signed.pdf" --signature 2
  ) >/dev/null
}

# shot PICTURE FILE STEP...: runs the app on FILE, takes the steps, and saves the window as PICTURE. The
# app's settings are a copy of SETTINGS (default OUT/cfg).
shot() {
  local picture=$1 file=$2
  shift 2
  local cfg steps status=0
  cfg=$(mktemp -d "$OUT/cfg-XXXX")
  cp -r "${SETTINGS:-$OUT/cfg}/." "$cfg/"
  steps=$(mktemp "$OUT/steps-XXXX")
  printf '%s\n' "$@" > "$steps"
  # The desktop's bus at a socket that isn't there: the app can't read the desktop's accent or settings.
  B=$B FILE=$file CFG=$cfg STEPS=$steps PICTURE=$picture W=$W H=$H NOBUS="unix:path=$cfg/no-such-bus" \
    xvfb-run -a -s "-screen 0 ${SCREEN}x24" bash -c '
    set -u
    env -u WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS=$NOBUS WINIT_X11_SCALE_FACTOR=2 \
      XDG_CONFIG_HOME=$CFG XDG_STATE_HOME=$CFG/state "$B" "$FILE" >/dev/null 2>&1 &
    app=$!
    win=
    for _ in $(seq 300); do
      win=$(xdotool search --name LimarePDF 2>/dev/null | head -1) && [ -n "$win" ] && break
      kill -0 $app 2>/dev/null || break
      sleep 0.1
    done
    [ -n "$win" ] || { echo "docshots.sh: the app showed no window" >&2; kill $app 2>/dev/null; exit 1; }
    sleep 1.5
    xdotool windowmove $win 0 0 windowsize $win $W $H
    sleep 1
    xdotool windowfocus --sync $win
    xdotool mousemove $((W / 2)) $((H - 12)) click 1
    sleep 0.5
    grab() { import -window root -crop ${W}x${H}+0+0 "$1"; }
    while IFS= read -r step; do
      case "$step" in
        k:*) xdotool key --clearmodifiers "${step#k:}"; sleep 0.4 ;;
        t:*) xdotool type --delay 30 "${step#t:}"; sleep 0.4 ;;
        c:*) IFS=, read x y <<< "${step#c:}"; xdotool mousemove $x $y; sleep 0.2; xdotool click 1; sleep 0.5 ;;
        m:*) IFS=, read mod x y <<< "${step#m:}"; xdotool mousemove $x $y; sleep 0.2; xdotool keydown $mod
             sleep 0.2; xdotool click 1; sleep 0.2; xdotool keyup $mod; sleep 0.5 ;;
        d:*) IFS=, read x1 y1 x2 y2 <<< "${step#d:}"; xdotool mousemove $x1 $y1; sleep 0.2; xdotool mousedown 1
             sleep 0.2; xdotool mousemove $(((x1 + x2) / 2)) $(((y1 + y2) / 2)); sleep 0.2
             xdotool mousemove $x2 $y2; sleep 0.2; xdotool mouseup 1; sleep 0.5 ;;
        h:*) IFS=, read x y <<< "${step#h:}"; xdotool mousemove $x $y; sleep 0.4 ;;
        w:*) IFS=, read x y n <<< "${step#w:}"; xdotool mousemove $x $y; sleep 0.2
             xdotool click --repeat $n --delay 50 5; sleep 0.5 ;;
        u:*) IFS=, read x y n <<< "${step#u:}"; xdotool mousemove $x $y; sleep 0.2
             xdotool click --repeat $n --delay 50 4; sleep 0.5 ;;
        p:*) sleep "${step#p:}" ;;
        s:*) sleep 0.6; grab "${PICTURE%.png}-${step#s:}.png" ;;
        *) echo "docshots.sh: unknown step: $step" >&2 ;;
      esac
    done < "$STEPS"
    sleep 0.8
    grab "$PICTURE"
    kill $app
  ' || status=$?
  rm -rf "$cfg" "$steps"
  return $status
}

# crop NAME GEOMETRY: OUT/raw/NAME.png cut to GEOMETRY (W x H + X + Y) as OUT/NAME.png.
crop() {
  magick "$OUT/raw/$1.png" -crop "$2" +repage -strip -define png:compression-level=9 "$OUT/$1.png"
}

if [ "${1:-}" = try ]; then
  [ $# -ge 2 ] || { echo "docshots.sh: try needs a file" >&2; exit 2; }
  file=$2
  shift 2
  shot "$OUT/try.png" "$file" "$@"
  exit
fi

# ---- The pictures ----
take() {
  case "$1" in
    # Comments view; Alex Kim's note selected on the page and a reply typed into its Reply box (left as a
    # draft); its card clicked, End, then Up four times to Alex Kim's highlight (which has Sam Lee's reply),
    # so the list scrolls with that card at the top; the card clicked; the page scrolled back up two notches.
    comments)
      shot "$OUT/raw/comments.png" "$OUT/fix/review.pdf" p:1.5 c:34,520 p:1 c:1399,940 p:1 c:1745,865 p:0.5 \
        "t:Done: it now says Results." p:0.5 c:270,1120 p:0.5 k:End p:0.5 k:Up p:0.4 k:Up p:0.4 k:Up p:0.4 \
        k:Up p:0.6 c:270,520 p:0.5 u:1000,800,2 p:0.5 h:1000,40 p:0.5
      crop comments 2000x966+0+282 ;;
    # Forms and sign > Fill and sign, the tool pinned, the first saved signature put down, then Date and
    # Tick from the Fill and sign menu (the notice above the page pushes the page down after the first
    # item), then the compact ribbon so the pane shows down to Marks.
    fill-and-sign)
      shot "$OUT/raw/fill-and-sign.png" "$OUT/fix/plain.pdf" p:1.5 c:1056,126 p:0.5 c:396,224 p:1 c:1866,333 \
        p:0.5 c:720,960 p:0.8 c:536,224 p:0.6 c:556,418 p:0.5 c:1030,1135 p:0.8 c:536,224 p:0.6 c:590,606 \
        p:0.5 c:1000,846 p:0.8 c:1950,124 p:0.8 h:1000,40 p:0.5
      crop fill-and-sign 1670x1004+330+228 ;;
    # The report's "Peak after the August storms" text box selected, its options in the inspector.
    inspector)
      shot "$OUT/raw/inspector.png" "$OUT/fix/field-report.pdf" p:1.5 c:1232,976 p:0.5
      crop inspector 1000x820+1000+282 ;;
    # Protect; "twelve monitoring stations" selected and Mark redaction > Mark for redaction; then Mark
    # redaction and a box dragged over the header's "Northern Water Survey" (left selected); the toast
    # left to go.
    redaction)
      shot "$OUT/raw/redaction.png" "$OUT/fix/plain.pdf" p:1.5 c:1258,126 p:0.5 d:504,665,749,665 p:0.5 \
        c:332,224 p:0.6 c:444,341 p:0.8 c:172,224 p:0.6 d:1212,488,1418,524 p:0.8 h:1000,40 p:4
      crop redaction 1670x720+330+282 ;;
    # The signed copy; Signatures in the banner, for the Digital signatures view.
    signatures-panel)
      signed
      SETTINGS=$OUT/signed-cfg shot "$OUT/raw/signatures-panel.png" "$OUT/fix/field-report-signed.pdf" p:2 \
        c:1368,348 p:1
      crop signatures-panel 1480x840+0+282 ;;
    *) echo "docshots.sh: no picture called $1" >&2; return 1 ;;
  esac
}

ALL="comments fill-and-sign inspector redaction signatures-panel"
for name in ${@:-$ALL}; do
  take "$name"
  echo "$OUT/$name.png"
done
