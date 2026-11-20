# /// script
# requires-python = ">=3.9"
# dependencies = ["pyte"]
# ///
r"""UX probe: drive the real sshi TUI in a pty and dump screens via pyte.

Backs docs/audit/2026-10-21-tui-mouse-ux-assessment.md (the "pty driver" in its intro).
Copied 2026-11-20 from gitignored scratch (docs/tmp/claude-scratch/2026-10-21-ux-probe/probe.py)
and made self-contained: config and state go to a temp dir instead of beside the script, and the
binary path is configurable. Needs a built binary (`cargo build`) and pyte (third-party VT
emulator; no repo build step or crate import). Unix only (pty).

Run (from the repo root):
  uv run docs/audit/2026-10-21-tui-mouse-ux-assessment/probe.py STEP...
STEP is either "dump:<title>" or a key string (python escapes allowed, e.g. '\x1b[B').
Env: SSHI_BIN (default target/debug/sshi), COLS (default 100), ROWS (default 30).
"""
import os, pty, select, struct, fcntl, termios, time, sys, pathlib, tempfile
import pyte

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[2]
BIN = pathlib.Path(os.environ.get("SSHI_BIN", ROOT / "target/debug/sshi"))
WORK = pathlib.Path(tempfile.mkdtemp(prefix="sshi-ux-probe-"))
STATE = WORK / "state"
STATE.mkdir(exist_ok=True)
CFG = WORK / "config.toml"
CFG.write_text(f'''[settings]
state_dir = "{STATE}"

[[host]]
name = "web-prod-1"
ssh_host = "192.0.2.10"
shell = "sh"
groups = ["web", "prod"]

[[host]]
name = "web-prod-2"
ssh_host = "192.0.2.11"
shell = "sh"
groups = ["web", "prod"]

[[host]]
name = "db-primary"
ssh_host = "192.0.2.20"
shell = "sh"
groups = ["db"]

[[check]]
name = "default"
enabled = ["online", "system_info", "memory", "disk"]

[[sync]]
name = "nginx-config"
paths = ["/etc/nginx/nginx.conf"]
''')

COLS, ROWS = int(os.environ.get("COLS", 100)), int(os.environ.get("ROWS", 30))
screen = pyte.Screen(COLS, ROWS)
stream = pyte.ByteStream(screen)

pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execv(str(BIN), [str(BIN), "-c", str(CFG)])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))


def pump(t=0.6):
    end = time.time() + t
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                return
            if b"\x1b[6n" in data:
                os.write(fd, b"\x1b[1;1R")
            stream.feed(data)


def dump(title):
    print(f"===== {title} =====")
    for line in screen.display:
        print(line.rstrip())


def send(keys, t=0.5):
    os.write(fd, keys.encode("latin-1"))
    pump(t)


pump(1.5)
for step in sys.argv[1:] or ["dump:start"]:
    if step.startswith("dump:"):
        dump(step[5:])
    else:
        send(step.encode("utf-8").decode("unicode_escape"))
send("q", 0.8)
