# The README demo. Re-render from the repo root with:
#
#     vhs contrib/demo.tape
#
# Needs vhs (github.com/charmbracelet/vhs) and ttyd on PATH, and `carrel`
# installed. Records in a staged /tmp/carrel-demo with scratch XDG dirs so
# no personal config, state, or filenames can appear in the recording.

Output assets/demo.gif

Set Shell "bash"
Set FontSize 20
Set Width 1200
Set Height 660
Set Padding 16
Set TypingSpeed 100ms

# --- staging, off camera -------------------------------------------------
# A stand-in docs folder: this repo's real public documents, plus a few
# short stubs so the home screen shows a realistic pile to read from.
Hide
Type "DEMO=/tmp/carrel-demo; rm -rf $DEMO; mkdir -p $DEMO/xdg/config $DEMO/xdg/state $DEMO/xdg/cache"
Enter
Type "cp README.md CONTRIBUTING.md CODE_OF_CONDUCT.md CHANGELOG.md $DEMO/"
Enter
Type 'for f in getting-started installation keybindings themes faq troubleshooting; do echo "# $f" > $DEMO/$f.md; done'
Enter
Type 'i=0; for f in troubleshooting faq themes keybindings installation getting-started CODE_OF_CONDUCT CONTRIBUTING README CHANGELOG; do touch -d "$((30 - i * 3)) days ago" $DEMO/$f.md; i=$((i+1)); done'
Enter
Type "export XDG_CONFIG_HOME=$DEMO/xdg/config XDG_STATE_HOME=$DEMO/xdg/state XDG_CACHE_HOME=$DEMO/xdg/cache"
Enter
Type "cd $DEMO && clear"
Enter
Show

# --- the demo ------------------------------------------------------------
Type "carrel"
Sleep 800ms
Enter
Sleep 2.5s

# type-to-filter down to the README
Type "i"
Sleep 700ms
Type@180ms "read"
Sleep 1.2s
Enter
Sleep 3s

# read a little
Type "j"
Sleep 400ms
Type "j"
Sleep 400ms
Type "j"
Sleep 700ms
Ctrl+D
Sleep 1.5s
Ctrl+D
Sleep 3.5s
