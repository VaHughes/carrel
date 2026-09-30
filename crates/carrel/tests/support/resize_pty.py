"""Live resize regression. Python stdlib only; launched by the Rust PTY test.

The small screen model handles the cursor/erase sequences emitted by ratatui.
Assertions inspect completed screens and actual clipboard/sidecar output, not
just strings that might have appeared in an earlier frame.
"""
import codecs
import fcntl
import os
import pty
import re
import select
import signal
import struct
import sys
import termios
import time
import unicodedata
from pathlib import Path


class Screen:
    def __init__(self, cols=80, rows=24):
        self.resize(cols, rows)
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def resize(self, cols, rows):
        previous = getattr(self, "cells", [])
        self.cols, self.rows = cols, rows
        self.cells = [[" "] * cols for _ in range(rows)]
        for y, row in enumerate(previous[:rows]):
            self.cells[y][:min(cols, len(row))] = row[:cols]
        self.x = self.y = 0

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        while self.pending:
            s = self.pending
            if s[0] == "\x1b":
                if len(s) < 2:
                    break
                if s[1] == "[":
                    match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", s)
                    if not match:
                        break
                    args, _, cmd = match.groups()
                    self.pending = s[match.end():]
                    if args.startswith("?"):
                        continue
                    nums = [int(n or 0) for n in args.split(";")] if args else [0]
                    n = nums[0] or 1
                    if cmd in "Hf":
                        self.y = max(0, nums[0] - 1)
                        self.x = max(0, (nums[1] if len(nums) > 1 else 1) - 1)
                    elif cmd == "A": self.y = max(0, self.y - n)
                    elif cmd == "B": self.y = min(self.rows - 1, self.y + n)
                    elif cmd == "C": self.x = min(self.cols - 1, self.x + n)
                    elif cmd == "D": self.x = max(0, self.x - n)
                    elif cmd == "G": self.x = n - 1
                    elif cmd == "d": self.y = n - 1
                    elif cmd == "J" and nums[0] in (2, 3):
                        self.cells = [[" "] * self.cols for _ in range(self.rows)]
                    elif cmd == "K" and self.y < self.rows:
                        start, end = (0, self.cols) if nums[0] == 2 else ((0, self.x + 1) if nums[0] == 1 else (self.x, self.cols))
                        self.cells[self.y][start:end] = [" "] * (end - start)
                    continue
                if s[1] in "]P_":
                    ends = [i for i in (s.find("\x07", 2), s.find("\x1b\\", 2)) if i >= 0]
                    if not ends: break
                    end = min(ends)
                    self.pending = s[end + (1 if s[end] == "\x07" else 2):]
                    continue
                self.pending = s[2:]
                continue
            c = s[0]
            self.pending = s[1:]
            if c == "\r": self.x = 0
            elif c == "\n": self.y = min(self.rows - 1, self.y + 1)
            elif c == "\b": self.x = max(0, self.x - 1)
            elif ord(c) >= 32:
                width = 0 if unicodedata.combining(c) else (2 if unicodedata.east_asian_width(c) in "WF" else 1)
                if 0 <= self.y < self.rows and 0 <= self.x < self.cols:
                    self.cells[self.y][self.x] = c
                self.x += width

    def text(self):
        return "\n".join("".join(row) for row in self.cells)


def main(binary, scratch, from_home=False):
    root = Path(scratch)
    doc = root / "reader.md"
    doc.write_text("selectable text for copying.\n\nneedle first.\n\nneedle second.\n\n" + "reading paragraph.\n\n" * 35)
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(root)
        for name in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            os.environ[name] = str(root / name)
        os.environ["TERM"] = "xterm-256color"
        os.environ.pop("NO_COLOR", None)
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        os.execv(binary, [binary, str(root if from_home else doc)])
    screen = Screen()
    raw = bytearray()

    def pump(duration=.12):
        end = time.monotonic() + duration
        while time.monotonic() < end:
            if select.select([fd], [], [], min(.02, max(0, end-time.monotonic())))[0]:
                try: data = os.read(fd, 65536)
                except OSError: return
                if not data: return
                raw.extend(data)
                screen.feed(data)

    def expect(text):
        end = time.monotonic() + 4
        while text not in screen.text() and time.monotonic() < end:
            pump(.04)
        assert text in screen.text(), f"missing {text!r}:\n{screen.text()}"

    def send(keys):
        os.write(fd, keys.encode())
        pump()

    def resize(cols, rows, immediate="", delay=.16):
        screen.resize(cols, rows)
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
        os.kill(pid, signal.SIGWINCH)
        if immediate: os.write(fd, immediate.encode())
        pump(delay)

    try:
        if from_home:
            expect("reader.md")
            send("\r")
        expect("selectable")
        send("/a-long-query-with-END")
        for size in ((24,6),(12,5),(80,24),(24,6)):
            resize(*size)
            expect("END")
        expect("no matches")
        send("\x1b")
        send("/needle\r")
        expect("1 of 2")
        for size in ((80,24),(20,5),(40,8),(24,6)):
            resize(*size)
        expect("1 of 2")
        send("n")
        expect("2 of 2")
        send("\x1bgg")
        resize(80,24)
        expect("selectable")
        # Select text, resize with selection active, and release in the new frame.
        send("\x1b[<0;3;2M\x1b[<32;12;2M")
        resize(24,6)
        send("\x1b[<0;12;2m")
        assert b"\x1b]52;c;" in raw, "selection must still copy through OSC 52"
        send("\x1b")
        # Notes preserve drafts and refuse invisible input, including bracketed paste.
        resize(80,24)
        send("a")
        expect("Note")
        send("draft-resize")
        resize(8,2,"X\r")
        send("\x1b[200~hidden-paste\x1b[201~")
        resize(24,8)
        expect("draft-resize")
        assert "hidden-paste" not in screen.text(), screen.text()
        send("\r")
        files = list((root / "XDG_STATE_HOME").rglob("*.notes"))
        assert files, "note must save after growing"
        saved = files[0].read_text()
        assert "draft-resize" in saved and "hidden-paste" not in saved and "draft-resizeX" not in saved, saved
        # Exercise menu selection and stale pointer coordinates across rapid resizes.
        send("\x1b")
        resize(24,6)
        send("\x1b[<0;24;6M\x1b[<0;24;6m")
        expect("Back")
        send("\x1b[B\x1b[B")
        before_burst = screen.text()
        for size in ((12,4),(80,24),(8,2),(24,6)):
            resize(*size, delay=.005)
        pump(.2)
        assert screen.text() == before_burst, f"menu lost cells after resize burst:\n{screen.text()}"
        expect("Back")
        send("\x1b")
        # The new launcher position must work; closing it must not quit the reader.
        send("\x1b[<0;24;6M\x1b[<0;24;6m")
        expect("Back")
        send("\x1b")
        send("Q")
        end = time.monotonic() + 4
        while time.monotonic() < end:
            child, status = os.waitpid(pid, os.WNOHANG)
            if child:
                assert os.waitstatus_to_exitcode(status) == 0, bytes(raw).decode(errors="replace")
                assert b"\x1b[?1049l" in raw, "terminal must be restored"
                print("live resize: search, selection, notes, menus, terminal restoration passed")
                return
            pump(.05)
        raise AssertionError("reader did not quit")
    finally:
        try: os.kill(pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(pid, 0)
        except ChildProcessError: pass
        os.close(fd)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], "--home" in sys.argv[3:])
