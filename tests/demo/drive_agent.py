#!/usr/bin/env python3
"""Drives `helm run` in a pseudo-terminal and answers each approval prompt from an explicit
policy. Nothing is approved implicitly: every prompt is matched against --allow patterns
(regular expressions over the prompt text); anything else is denied. Writes a transcript.

usage: drive_agent.py --transcript FILE [--allow REGEX]... [--cancel-after SECONDS] -- helm args...
"""
import argparse, os, pty, re, select, signal, sys, time

ap = argparse.ArgumentParser()
ap.add_argument("--transcript", required=True)
ap.add_argument("--allow", action="append", default=[])
ap.add_argument("--cancel-after", type=float)
ap.add_argument("--timeout", type=float, default=300)
ap.add_argument("cmd", nargs=argparse.REMAINDER)
a = ap.parse_args()
cmd = a.cmd[1:] if a.cmd and a.cmd[0] == "--" else a.cmd

pid, fd = pty.fork()
if pid == 0:
    os.execvp(cmd[0], cmd)
buf, decisions, start = "", [], time.time()
prompt = "Approve this exact action once? [y/N] "
cancelled = False
with open(a.transcript, "w") as log:
    while True:
        if time.time() - start > a.timeout:
            os.kill(pid, signal.SIGKILL); log.write("\n[driver] timeout\n"); break
        if a.cancel_after and not cancelled and time.time() - start > a.cancel_after:
            os.kill(pid, signal.SIGINT); cancelled = True; log.write("\n[driver] SIGINT sent\n")
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                data = os.read(fd, 4096).decode(errors="replace")
            except OSError:
                break
            if not data:
                break
            buf += data; log.write(data); log.flush()
            while buf.count(prompt) > len(decisions):
                idx = [m.start() for m in re.finditer(re.escape(prompt), buf)][len(decisions)]
                block = buf[max(0, buf.rfind("Approval required:", 0, idx)):idx]
                ok = any(re.search(p, block) for p in a.allow)
                decisions.append(ok)
                os.write(fd, b"y\n" if ok else b"n\n")
                log.write(f"\n[driver] {'APPROVED' if ok else 'DENIED'}\n")
        else:
            try:
                done, status = os.waitpid(pid, os.WNOHANG)
            except ChildProcessError:
                break
            if done:
                break
    try:
        _, status = os.waitpid(pid, 0)
        code = os.waitstatus_to_exitcode(status)
    except ChildProcessError:
        code = -1
    log.write(f"\n[driver] exit={code} approvals={decisions}\n")
print(f"exit={code} approvals={decisions}")
sys.exit(0)
