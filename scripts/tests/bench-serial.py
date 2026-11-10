"""Talks to the bench board over its USB serial port.

Run it with esptool's Python, which carries pyserial:

    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 reset
    uvx --from esptool@5.6.0 python scripts/tests/bench-serial.py --port /dev/ttyUSB0 \\
        watch --seconds 60 --until 'sta ip:' --fail-on-crash

`watch` prints the board's log with its colour codes removed as it arrives.
It exits 0 when a line matches --until, printing that line last; 1 when the
time runs out first; 2 when --fail-on-crash is given and a line holds crash
text; 3 on a bad command line. `--from-file FILE` reads a recorded log
instead of the port, and the end of the file counts as the time running
out. docs/testing.md describes the bench files.
"""

import argparse
import re
import sys
import time

BAUD = 115200
RESET_PULSE_S = 0.1
READ_TIMEOUT_S = 0.2
ANSI = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
CRASH_TEXT = (
    "Guru Meditation Error",
    "Backtrace:",
    "abort()",
    "Task watchdog got triggered",
)
MATCHED, TIMED_OUT, CRASHED, MISUSED = 0, 1, 2, 3


class Parser(argparse.ArgumentParser):
    """Exits 3 on a bad command line, so that 2 always means a crash."""

    def error(self, message):
        self.print_usage(sys.stderr)
        self.exit(MISUSED, f"{self.prog}: {message}\n")


def open_port(port):
    import serial

    link = serial.Serial()
    link.port = port
    link.baudrate = BAUD
    link.timeout = READ_TIMEOUT_S
    # Opening with both lines released keeps the board running.
    link.dtr = False
    link.rts = False
    link.open()
    return link


def reset(args):
    if args.from_file:
        print(f"reset: nothing to do for {args.from_file}")
        return 0
    link = open_port(args.port)
    link.rts = True
    time.sleep(RESET_PULSE_S)
    link.rts = False
    link.close()
    print(f"reset: pulsed RTS on {args.port}")
    return 0


def file_lines(path):
    with open(path, "rb") as recorded:
        yield from recorded


def port_lines(port, seconds):
    deadline = time.monotonic() + seconds
    pending = b""
    with open_port(port) as link:
        while time.monotonic() < deadline:
            pending += link.read(4096)
            *complete, pending = pending.split(b"\n")
            for line in complete:
                yield line + b"\n"
    if pending:
        yield pending


def plain(raw):
    return ANSI.sub("", raw.decode("utf-8", errors="replace")).rstrip("\r\n")


def outcome(line, until, fail_on_crash):
    if until.search(line):
        return MATCHED
    if fail_on_crash and any(text in line for text in CRASH_TEXT):
        return CRASHED
    return None


def watch(args):
    until = re.compile(args.until)
    lines = file_lines(args.from_file) if args.from_file else port_lines(args.port, args.seconds)
    log = open(args.log, "a", encoding="utf-8") if args.log else None
    result = TIMED_OUT
    for raw in lines:
        line = plain(raw)
        print(line, flush=True)
        if log:
            log.write(line + "\n")
        result = outcome(line, until, args.fail_on_crash)
        if result is not None:
            break
    if log:
        log.close()
    return TIMED_OUT if result is None else result


def parser():
    top = Parser(
        prog="bench-serial.py",
        description="Reset the bench board or watch its serial log.",
    )
    source = top.add_mutually_exclusive_group(required=True)
    source.add_argument("--port", help="the board's serial port, such as /dev/ttyUSB0")
    source.add_argument("--from-file", metavar="FILE", help="read a recorded log instead of a port")
    commands = top.add_subparsers(dest="command", required=True)

    commands.add_parser("reset", help="pulse RTS to reset the board").set_defaults(run=reset)

    watching = commands.add_parser(
        "watch",
        help="print the log until a pattern appears",
        description="Exit 0 when a line matches --until (printed last), "
        "1 when the time runs out, 2 on crash text with --fail-on-crash.",
    )
    watching.add_argument("--seconds", type=float, required=True, help="how long to wait")
    watching.add_argument("--until", required=True, metavar="REGEX", help="the line to wait for")
    watching.add_argument("--log", metavar="FILE", help="also append the log to FILE")
    watching.add_argument(
        "--fail-on-crash", action="store_true", help="exit 2 when a line holds crash text"
    )
    watching.set_defaults(run=watch)
    return top


def main():
    args = parser().parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
