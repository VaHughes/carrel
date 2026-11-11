# Run from the repository root after `cargo build`:
#     vhs contrib/screenshot.tape
# Requires vhs, ttyd, ffmpeg, Chromium and JetBrainsMono Nerd Font (or change FontFamily).
# Only the checked-in sample and a temporary config/state directory are shown.

Set Shell "bash"
Set FontFamily "JetBrainsMono Nerd Font"
Set FontSize 20
Set Width 1200
Set Height 850
Set Padding 16
Set TypingSpeed 1ms

Hide
Type 'CARREL_CAPTURE_ROOT=$(mktemp -d); cp contrib/preview.md "$CARREL_CAPTURE_ROOT/field-notes.md"; export XDG_CONFIG_HOME="$CARREL_CAPTURE_ROOT/config" XDG_STATE_HOME="$CARREL_CAPTURE_ROOT/state" XDG_CACHE_HOME="$CARREL_CAPTURE_ROOT/cache"; mkdir -p "$XDG_CONFIG_HOME/carrel"; echo "theme = dark" > "$XDG_CONFIG_HOME/carrel/config"; unset NO_COLOR; target/debug/carrel "$CARREL_CAPTURE_ROOT/field-notes.md"'
Enter
Sleep 2s
Show
Sleep 500ms
Screenshot assets/screenshot.png
Hide
Type 'q'
Sleep 200ms
Type 'rm -rf -- "$CARREL_CAPTURE_ROOT"'
Enter
