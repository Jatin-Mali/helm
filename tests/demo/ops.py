#!/usr/bin/env python3
"""Operations demonstration against an isolated HELM state and a harmless local fixture.

usage: ops.py HELM_BINARY WORKDIR [--evidence FILE]

Fixture: a loopback HTTP "service" whose health depends on app.conf (mode=broken -> 500).
Flow: register -> collect (unhealthy service becomes a finding) -> explain -> review a two-step
plan -> DENY step 1, APPROVE step 2 (a file write) -> verify via HTTP -> new process still
shows the change set -> roll back -> verify original bytes -> user edit makes rollback FAIL
visibly -> a failed collector is reported as not-successful. Nothing outside WORKDIR is touched.
"""
import json, os, pty, re, select, sqlite3, subprocess, sys, time, urllib.request, urllib.error, signal, uuid, socket

helm, work = sys.argv[1], os.path.abspath(sys.argv[2])
evidence_path = sys.argv[sys.argv.index("--evidence") + 1] if "--evidence" in sys.argv else None
os.makedirs(work, exist_ok=True)
env = dict(os.environ, XDG_CONFIG_HOME=f"{work}/config", XDG_DATA_HOME=f"{work}/data", XDG_CACHE_HOME=f"{work}/cache")
for k in list(env):
    if k.endswith("_API_KEY"):
        del env[k]
log = []

def say(msg):
    print(msg, flush=True); log.append(msg)

def check(cond, msg):
    say(("PASS  " if cond else "FAIL  ") + msg)
    if not cond:
        finish(1)

def finish(code):
    if evidence_path:
        open(evidence_path, "w").write("\n".join(log) + "\n")
    server and server.terminate()
    sys.exit(code)

def run(*args, check_ok=True, extra_env=None):
    r = subprocess.run([helm, *args], env={**env, **(extra_env or {})}, capture_output=True, text=True, timeout=180)
    return r

def http(port):
    try:
        return urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=3).status
    except urllib.error.HTTPError as e:
        return e.code

def free_port():
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p

def pty_run(args, answers, prompt_re):
    """Run helm in a pty, answering each prompt matching prompt_re from the explicit list."""
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(work)
        os.execve(helm, [helm, *args], env)
    buf, given, deadline = "", 0, time.time() + 120
    while time.time() < deadline:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                data = os.read(fd, 4096).decode(errors="replace")
            except OSError:
                break
            if not data:
                break
            buf += data
            while len(re.findall(prompt_re, buf)) > given and given < len(answers):
                os.write(fd, (answers[given] + "\n").encode()); given += 1
        else:
            if os.waitpid(pid, os.WNOHANG)[0]:
                pid = None
                break
    code = None
    if pid:
        try:
            _, st = os.waitpid(pid, 0); code = os.waitstatus_to_exitcode(st)
        except ChildProcessError:
            pass
    return code, buf.replace("\r", "")

port = free_port()
app = f"{work}/app"; os.makedirs(app, exist_ok=True)
conf = f"{app}/app.conf"
open(conf, "w").write("mode=broken\n")
open(f"{app}/server.py", "w").write('''import http.server, sys
conf = sys.argv[2]
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        mode = open(conf).read().strip()
        self.send_response(200 if mode == "mode=ok" else 500); self.end_headers(); self.wfile.write(mode.encode())
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
''')
server = subprocess.Popen([sys.executable, f"{app}/server.py", str(port), conf])
time.sleep(1)
check(http(port) == 500, "fixture service is unhealthy before any change (HTTP 500)")

r = run("service", "add", "--id", "demo-app", "--name", "demo-app", "--type", "http-endpoint", "--url", f"http://127.0.0.1:{port}/health", "--expected-status", "200")
check(r.returncode == 0, "service registered in isolated state")

# Collection: findings appear only after repeated failures (3 consecutive checks).
finding = None
for _ in range(4):
    r = run("monitor", "--format", "json", "--profile", "quick")
    report = json.loads(r.stdout)
    hits = [f for f in report["findings"] if "demo-app" in f["title"]]
    finding = hits[0] if hits else None
check(finding is not None, "unhealthy service surfaced as a finding with evidence: " + (finding["title"] if finding else "-"))
check(any(v != "successful" for v in report["coverage"].values()), "report states which domains were NOT successfully observed (coverage: %s)" % sorted({v for v in report["coverage"].values()}))
r = run("explain", finding["id"])
check("http status=500" in r.stdout, "explain shows the evidence (http status=500)")

# Reviewed plan with two steps: a harmless shell step (to be denied) and a reversible file write.
db = f"{work}/data/helm/helm.db"
steps = [
  {"title": "Announce restart (will be denied)", "command": {"tool": "shell", "input": {"command": f"touch {work}/DENIED_STEP_RAN"}, "command_text": f"touch {work}/DENIED_STEP_RAN",
     "expected_effect": "create a marker file", "risk": "low", "blast_radius": "unknown", "rollback": "unsupported", "verification": []}, "hypothesis_id": None, "expected_output": None, "interpretation_guide": None},
  {"title": "Set mode=ok in app.conf", "command": {"tool": "fs_write", "input": {"path": conf, "content": "mode=ok\n"}, "command_text": None,
     "expected_effect": "service reads mode=ok and reports healthy", "risk": "medium", "blast_radius": {"file": conf}, "rollback": {"available": "restore previous file bytes"}, "verification": []}, "hypothesis_id": None, "expected_output": None, "interpretation_guide": None},
]
plan_id = "plan-" + uuid.uuid4().hex[:8]
conn = sqlite3.connect(db)
now = int(time.time())
conn.execute("INSERT INTO troubleshooting_plans (id, source, snapshot_id, finding_id, finding_fingerprint, hypotheses_json, read_only_steps_json, proposed_fix_steps_json, approval_required, created_at, updated_at, verdict_summary, narrative_summary, dashboard_plan_status, generation_error, verification_steps_json, reproduction_steps_json) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,'','[]','[]')",
  (plan_id, "finding:" + finding["id"], report["snapshot_id"], finding["id"], finding.get("fingerprint", ""), "[]", "[]", json.dumps({"schema_version": 1, "steps": steps}), 1, now, now, "demo", "demo", "ready"))
conn.commit(); conn.close()

code, out = pty_run(["apply-plan", plan_id], ["n", "y"], r"Execute this step\? \[y/N/! \(execute all remaining\)\] ")
check(code == 0, "apply-plan completed (exit %s)" % code)
check(not os.path.exists(f"{work}/DENIED_STEP_RAN"), "denied step did not run")
check(open(conf).read() == "mode=ok\n", "approved step changed the file")
check(http(port) == 200, "service is healthy after the approved change (HTTP 200)")

# Restart: a fresh process still shows the durable change set.
r = run("change-set", "list")
m = re.search(r"\b(cs-[0-9a-f-]+|[0-9a-f]{8}-[0-9a-f-]{27})\b", r.stdout)
check(m is not None, "change set persisted and visible from a new process")
cs = m.group(1)
r = run("change-set", "show", cs)
check("fs_write" in r.stdout or "app.conf" in r.stdout, "change set records the exact operation")

# Rollback restores bytes; the service is unhealthy again.
r = run("change-set", "rollback", cs, "--yes")
check(r.returncode == 0 and open(conf).read() == "mode=broken\n", "rollback restored the original bytes")
check(http(port) == 500, "service reverted to its original (unhealthy) state")

# A newer user edit makes restoration fail visibly instead of overwriting it.
plan2 = "plan-" + uuid.uuid4().hex[:8]
conn = sqlite3.connect(db)
conn.execute("INSERT INTO troubleshooting_plans (id, source, snapshot_id, finding_id, finding_fingerprint, hypotheses_json, read_only_steps_json, proposed_fix_steps_json, approval_required, created_at, updated_at, verdict_summary, narrative_summary, dashboard_plan_status, generation_error, verification_steps_json, reproduction_steps_json) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,'','[]','[]')",
  (plan2, "finding:" + finding["id"], report["snapshot_id"], finding["id"], finding.get("fingerprint", ""), "[]", "[]", json.dumps({"schema_version": 1, "steps": steps[1:]}), 1, now, now, "demo", "demo", "ready"))
conn.commit(); conn.close()
before = set(re.findall(r"\b[0-9a-f]{8}-[0-9a-f-]{27}\b|\bcs-[0-9a-f-]+\b", run("change-set", "list").stdout))
code, out = pty_run(["apply-plan", plan2], ["y"], r"Execute this step\? \[y/N/! \(execute all remaining\)\] ")
check(code == 0 and open(conf).read() == "mode=ok\n", "second reversible change applied")
after = set(re.findall(r"\b[0-9a-f]{8}-[0-9a-f-]{27}\b|\bcs-[0-9a-f-]+\b", run("change-set", "list").stdout))
second = sorted(after - before)
check(len(second) == 1, "second change set recorded")
open(conf, "w").write("mode=edited-by-user\n")
r = run("change-set", "rollback", second[0], "--yes")
message = (r.stdout + r.stderr).strip()
check(open(conf).read() == "mode=edited-by-user\n", "rollback did not overwrite the newer user edit")
check(r.returncode != 0 and re.search(r"fail|changed|newer|differs|mismatch", message, re.I) is not None, "failed restoration is reported as failed (exit %s): %s" % (r.returncode, message.splitlines()[-1][:140] if message else "-"))

# A collector that cannot run is unknown/failed, never healthy.
r = run("monitor", "--format", "json", "--profile", "quick", extra_env={"PATH": "/nonexistent"})
rep = json.loads(r.stdout)
bad = [d for d, s in rep["coverage"].items() if s in ("failed", "unknown", "unsupported")]
check(len(bad) >= 5 and rep["collector_errors"], "with core tools missing, %d domains are failed/unknown/unsupported (not healthy)" % len(bad))

say("ALL OPERATIONS CHECKS PASSED")
finish(0)
