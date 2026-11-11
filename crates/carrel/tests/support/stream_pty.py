"""Exercise the actual stdin decoder thread with malformed bytes in a real PTY."""
import fcntl
import os
import pty
import select
import signal
import struct
import sys
import termios
import threading
import time
from pathlib import Path

sys.dont_write_bytecode = True
from resize_pty import Screen


def main(binary, scratch):
    root = Path(scratch)
    read_fd, write_fd = os.pipe()
    pid, terminal = pty.fork()
    if pid == 0:
        os.close(write_fd)
        os.chdir(root)
        for name in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
            os.environ[name] = str(root / name)
        os.environ["TERM"] = "xterm-256color"
        os.environ.pop("NO_COLOR", None)
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
        os.dup2(read_fd, 0)
        os.close(read_fd)
        os.execv(binary, [binary])
    os.close(read_fd)
    screen = Screen()
    raw = bytearray()

    def produce():
        try:
            # More invalid bytes than a recursive decoder can tolerate,
            # followed by valid Unicode and a uniquely identifiable ending.
            pending = memoryview(b"\xff" * 65536 + "\n\nCafé 界\n\nSTREAM_COMPLETE\n".encode())
            while pending:
                pending = pending[os.write(write_fd, pending):]
        except BrokenPipeError:
            pass  # A crashed decoder is reported from the child's status below.
        finally:
            os.close(write_fd)

    producer = threading.Thread(target=produce, daemon=True)
    producer.start()

    def pump():
        if select.select([terminal], [], [], .05)[0]:
            try:
                data = os.read(terminal, 65536)
            except OSError:
                return
            raw.extend(data)
            screen.feed(data)

    try:
        end = time.monotonic() + 15
        while "(stdin" not in screen.text() and time.monotonic() < end:
            pump()
        assert "(stdin" in screen.text(), f"no reader screen: {raw!r}"
        # Before EOF, G keeps up with new chunks; after EOF it simply
        # navigates to the ending. Send one key, never flood the event queue.
        os.write(terminal, b"G")
        while "STREAM_COMPLETE" not in screen.text() and time.monotonic() < end:
            pump()
        assert "STREAM_COMPLETE" in screen.text(), f"lost or crashed input:\n{screen.text()}\n{raw[-2000:]!r}"
        assert "Café 界" in screen.text(), screen.text()
        assert "�" in screen.text(), "invalid bytes must remain visible as replacement characters"
        os.write(terminal, b"q")
        end = time.monotonic() + 5
        while time.monotonic() < end:
            pump()
            child, status = os.waitpid(pid, os.WNOHANG)
            if child:
                assert os.waitstatus_to_exitcode(status) == 0, f"child status {status}: {raw[-2000:]!r}"
                assert b"\x1b[?1049l" in raw, "terminal was not restored"
                print("stream PTY: malformed bytes, Unicode, ending, and clean exit passed")
                return
        raise AssertionError(f"reader did not exit:\n{screen.text()}\n{raw[-2000:]!r}")
    finally:
        try: os.kill(pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: os.waitpid(pid, 0)
        except ChildProcessError: pass
        os.close(terminal)
        producer.join(timeout=1)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
