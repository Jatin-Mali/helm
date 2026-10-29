#!/usr/bin/env python3
"""Two disposable SSH hosts (Docker): distinguishable observations; a dead host is UNKNOWN/STALE,
never replaced by local data.   usage: fleet.py HELM WORKDIR   (requires docker, ssh, ssh-keygen)"""
import json, os, shutil, subprocess, sys

helm, work = sys.argv[1], os.path.abspath(sys.argv[2])
here = os.path.dirname(os.path.abspath(__file__))
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
env = dict(os.environ, XDG_CONFIG_HOME=f"{work}/config", XDG_DATA_HOME=f"{work}/data", XDG_CACHE_HOME=f"{work}/cache")
for k in list(env):
    if k.endswith("_API_KEY"): del env[k]

def sh(*cmd, **kw):
    return subprocess.run(cmd, env=kw.pop("env", env), capture_output=True, text=True, timeout=kw.pop("timeout", 600), **kw)

def ok(cond, msg):
    print(("PASS  " if cond else "FAIL  ") + msg, flush=True)
    if not cond:
        sh("bash", f"{here}/fleet.sh", "down"); sys.exit(1)

if sh("docker", "version").returncode != 0:
    print("UNAVAILABLE: docker"); sys.exit(77)
r = sh("bash", f"{here}/fleet.sh", "up", f"{work}/keys", helm.replace("/helm", "/helmops") if helm.endswith("/helm") and os.path.exists(helm.replace("/helm", "/helmops")) else helm)
ok(r.returncode == 0, "two disposable SSH hosts started: " + r.stderr.strip()[-120:])
try:
    opts = f"-i {work}/keys/id_ed25519 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o BatchMode=yes"
    for n in "ab":
        ip = open(f"{work}/keys/helm-demo-{n}.ip").read().strip()
        r = sh(helm, "remote", "add", n, "--host", ip, "--user", "root", f"--ssh-opts={opts}")
        ok(r.returncode == 0, f"remote {n} registered ({ip})")
    r = sh(helm, "monitor", "--fleet", "--json")
    rows = {x["host"]: x for x in json.loads(r.stdout)}
    ok(all(rows[n]["status"] == "observed" for n in "ab"), "both hosts observed over SSH")
    ids = {n: rows[n]["last_known"]["identity"] for n in "ab"}
    ok(ids["a"] != ids["b"], "host identities are distinguishable (%s… vs %s…)" % (ids["a"][:10], ids["b"][:10]))
    local = sh(helm, "snapshot", "--json", "--profile", "quick")
    local_id = json.loads(local.stdout)["domains"]["collection"]["host_id"]
    ok(local_id is not None and all(identity and identity != local_id for identity in ids.values()),
       "local and remote collection identities are distinct")
    sh("docker", "stop", "helm-demo-b")
    r = sh(helm, "monitor", "--fleet", "--json")
    rows = {x["host"]: x for x in json.loads(r.stdout)}
    ok(rows["a"]["status"] == "observed", "surviving host is still observed")
    ok(rows["b"]["status"] == "unavailable" and rows["b"].get("last_known") is None, "disconnected host is unavailable, not healthy")
    stale = rows["b"].get("stale_last_known") or {}
    ok(stale.get("identity") == ids["b"], "its last known data is kept, labeled stale, with the same identity")
    text = sh(helm, "monitor", "--fleet").stdout
    ok("UNKNOWN" in text and "STALE" in text, "text view says UNKNOWN / STALE")
    print("ALL FLEET CHECKS PASSED")
finally:
    sh("bash", f"{here}/fleet.sh", "down")
