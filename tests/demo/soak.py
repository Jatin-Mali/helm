#!/usr/bin/env python3
"""Prolonged monitoring run: daemon collects every INTERVAL seconds in isolated state.
Records RSS, database size and snapshot count over time plus single-collection latency.

usage: soak.py HELM WORKDIR [--minutes 10] [--interval 5] [--out FILE]
"""
import argparse, json, os, shutil, sqlite3, subprocess, sys, time

ap = argparse.ArgumentParser()
ap.add_argument("helm"); ap.add_argument("work")
ap.add_argument("--minutes", type=float, default=10); ap.add_argument("--interval", type=int, default=5)
ap.add_argument("--out")
a = ap.parse_args()
work = os.path.abspath(a.work); shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
env = dict(os.environ, XDG_CONFIG_HOME=f"{work}/config", XDG_DATA_HOME=f"{work}/data", XDG_CACHE_HOME=f"{work}/cache")
for k in list(env):
    if k.endswith("_API_KEY"): del env[k]
db = f"{work}/data/helm/helm.db"

def run(*args):
    return subprocess.run([a.helm, *args], env=env, capture_output=True, text=True, timeout=300)

t = time.time(); r = run("monitor", "--format", "json", "--profile", "standard"); lat = time.time() - t
assert r.returncode == 0, r.stderr
print(f"single standard collection: {lat:.2f}s", flush=True)
r = run("daemon", "start", "--interval", str(a.interval)); assert r.returncode == 0, r.stderr
time.sleep(3)
state = json.load(open(f"{work}/data/helm/daemon-state.json")); pid = state["pid"]

def rss_mb():
    for line in open(f"/proc/{pid}/status"):
        if line.startswith("VmRSS"): return int(line.split()[1]) / 1024

def dbsize():
    return sum(os.path.getsize(db + s) for s in ("", "-wal", "-shm") if os.path.exists(db + s)) / 1e6

samples, end = [], time.time() + a.minutes * 60
while time.time() < end:
    time.sleep(20)
    if not os.path.exists(f"/proc/{pid}"):
        print("daemon exited early", flush=True); break
    try:
        n = sqlite3.connect(db, timeout=5).execute("select count(*) from snapshots").fetchone()[0]
    except sqlite3.Error:
        n = -1
    samples.append((round((time.time() - (end - a.minutes * 60)) / 60, 1), round(rss_mb(), 1), round(dbsize(), 1), n))
    print("t=%.1fmin rss=%.1fMB db=%.1fMB snapshots=%d" % samples[-1], flush=True)

status = run("daemon", "status").stdout
run("daemon", "stop")
alive = os.path.exists(f"/proc/{pid}")
first, last = samples[0], samples[-1]
per_snapshot = (last[2] - first[2]) / max(last[3] - first[3], 1)
summary = {
    "minutes": a.minutes, "interval_s": a.interval, "single_collection_s": round(lat, 2),
    "rss_start_mb": first[1], "rss_end_mb": last[1], "rss_growth_mb": round(last[1] - first[1], 1),
    "db_start_mb": first[2], "db_end_mb": last[2], "snapshots": last[3], "mb_per_snapshot": round(per_snapshot, 3),
    "projected_mb_at_1000_snapshot_cap": round(per_snapshot * 1000, 1), "daemon_stopped_cleanly": not alive,
}
print(json.dumps(summary, indent=2)); print(status)
if a.out: json.dump({"summary": summary, "samples": samples}, open(a.out, "w"), indent=1)
sys.exit(0 if (not alive and last[3] > first[3]) else 1)
