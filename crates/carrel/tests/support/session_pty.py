"""Home is still the same reading session, whichever entry point opened it."""
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import time
from pathlib import Path

sys.dont_write_bytecode = True
from resize_pty import Screen


def main(binary, scratch, mode):
    root = Path(scratch)
    (root / "book.md").write_text("# Session document\n\nSESSION_DOCUMENT\n")
    (root / "HOME_RETURN_MARKER.md").write_text("# Another book\n")
    pipe = os.pipe() if mode == "pipe" else None
    pid, terminal = pty.fork()
    if pid == 0:
        os.chdir(root)
        for name in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            os.environ[name] = str(root / name)
        os.environ["TERM"] = "xterm-256color"
        os.environ.pop("NO_COLOR", None)
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        if pipe:
            os.close(pipe[1])
            os.dup2(pipe[0], 0)
            os.close(pipe[0])
        args = [] if mode == "pipe" else (["--tutorial"] if mode == "tutorial" else ["book.md"])
        os.execv(binary, [binary, *args])
    if pipe:
        os.close(pipe[0])
        os.write(pipe[1], b"# Piped document\n\nA document from stdin.\n")
        os.close(pipe[1])
    screen, raw = Screen(), bytearray()

    def pump():
        if select.select([terminal], [], [], .03)[0]:
            try:
                data = os.read(terminal, 65536)
            except OSError:
                time.sleep(.01)
                return
            raw.extend(data)
            screen.feed(data)

    def wait_for(predicate, message):
        deadline = time.monotonic() + 8
        while not predicate() and time.monotonic() < deadline:
            pump()
        assert predicate(), f"{mode}: {message}\n{screen.text()}\n{raw[-1000:]!r}"

    def footer(word):
        return word in "".join(screen.cells[-1])

    def send(data):
        os.write(terminal, data)

    try:
        wait_for(lambda: "document" in screen.text(), "initial document did not open")
        send(b"/document\r")
        wait_for(lambda: footer("found"), "search was not accepted")
        send(b"\x1b")
        wait_for(lambda: footer("reading"), "search did not clear")
        send(b"\x1b[<0;1;23M\x1b[<0;1;23m")
        wait_for(lambda: footer("browse") and "HOME_RETURN_MARKER" in screen.text(),
                 "Home must open an interactive file list")
        send(b"ibook")
        wait_for(lambda: footer("filter") and "book" in screen.text(), "file filter did not open")
        send(b"\r")
        wait_for(lambda: footer("reading") and "SESSION_DOCUMENT" in screen.text(),
                 "the home list must open a document")
        send(b"/\x1b[A")
        wait_for(lambda: "/document" in screen.text(), "search history was lost on the way home")
        send(b"\x1b")
        wait_for(lambda: footer("reading"), "recalled search did not close")
        send(b"Q")
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            pump()
            child, status = os.waitpid(pid, os.WNOHANG)
            if child:
                assert os.waitstatus_to_exitcode(status) == 0, f"child status {status}"
                assert b"\x1b[?1049l" in raw, "terminal was not restored"
                print(f"{mode}: Home, file opening, session search recall, and clean exit passed")
                return
        raise AssertionError("reader did not quit")
    finally:
        try: os.kill(pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(pid, 0)
        except ChildProcessError: pass
        os.close(terminal)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3])
