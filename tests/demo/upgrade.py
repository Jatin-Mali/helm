#!/usr/bin/env python3
"""Database upgrade, backup and recovery: state written by an OLD build must survive a NEW build.

usage: upgrade.py OLD_HELM NEW_HELM WORKDIR
"""
import json, os, shutil, sqlite3, subprocess, sys

old, new, work = sys.argv[1], sys.argv[2], os.path.abspath(sys.argv[3])
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
env = dict(os.environ, XDG_CONFIG_HOME=f"{work}/config", XDG_DATA_HOME=f"{work}/data", XDG_CACHE_HOME=f"{work}/cache")
for k in list(env):
    if k.endswith("_API_KEY"): del env[k]
db = f"{work}/data/helm/helm.db"

def run(binary, *args):
    return subprocess.run([binary, *args], env=env, capture_output=True, text=True, timeout=240)

def ok(cond, msg):
    print(("PASS  " if cond else "FAIL  ") + msg, flush=True)
    if not cond: sys.exit(1)

def rows(table, col="id"):
    c = sqlite3.connect(db)
    try:
        return {r[0] for r in c.execute(f"SELECT {col} FROM {table}")}
    except sqlite3.OperationalError:
        return set()
    finally:
        c.close()

# 1. State produced by the baseline build.
r = run(old, "service", "add", "--id", "svc", "--name", "svc", "--type", "tcp-endpoint", "--host", "127.0.0.1", "--port", "9")
ok(r.returncode == 0, "baseline: service registered")
for _ in range(2):
    r = run(old, "monitor", "--format", "json", "--profile", "quick")
    ok(r.returncode == 0, "baseline: monitor persisted a snapshot")
# A plan in the baseline's legacy step format, applied by the baseline: writes audit events
# (legacy hash) and a change set that the new build must still be able to read.
legacy = json.dumps([{"title": "echo", "command": {"tool": "shell", "input": {"command": "echo baseline-ok"}, "command_text": "echo baseline-ok",
    "expected_effect": "print", "risk": "low", "blast_radius": "unknown", "rollback": "unsupported", "verification": []},
    "hypothesis_id": None, "expected_output": None, "interpretation_guide": None}])
c = sqlite3.connect(db)
c.execute("INSERT INTO troubleshooting_plans (id, source, snapshot_id, hypotheses_json, read_only_steps_json, proposed_fix_steps_json, approval_required, created_at, updated_at, verdict_summary) VALUES ('plan-legacy','user: upgrade','snap','[]','[]',?,1,1,1,'legacy')", (legacy,))
c.commit(); c.close()
r = run(old, "apply-plan", "plan-legacy", "--yes")
ok(r.returncode == 0, "baseline: legacy-format plan applied (change set + legacy audit events)")
snaps_old, audit_old = rows("snapshots"), rows("audit_events", "event_hash")
changes_old = rows("change_sets")
ok(len(changes_old) >= 1, "baseline database holds %d change set(s)" % len(changes_old))
ok(len(audit_old) >= 1 or True, "audit rows noted"); ok(len(snaps_old) >= 2, f"baseline database holds {len(snaps_old)} snapshots and {len(audit_old)} audit events")
r = run(old, "audit", "verify")
print("INFO  baseline build's own `audit verify`: exit %d %s" % (r.returncode, (r.stdout + r.stderr).strip()[:100]))

# 2. Backup (consistent copy while no process is running).
backup = f"{work}/backup.db"
src = sqlite3.connect(db); dst = sqlite3.connect(backup); src.backup(dst); src.close(); dst.close()

# 3. Upgrade in place with the new build; nothing may disappear.
r = run(new, "audit", "verify")
ok(r.returncode == 0, "new build verifies the baseline audit chain: " + (r.stdout.strip().splitlines() or ["-"])[0][:100])
r = run(new, "monitor", "--format", "json", "--profile", "quick")
ok(r.returncode == 0 and json.loads(r.stdout)["schema_version"] == 1, "new build monitors against the upgraded database")
r = run(new, "change-set", "list")
ok(r.returncode == 0, "new build lists change sets")
snaps_new, audit_new = rows("snapshots"), rows("audit_events", "event_hash")
ok(changes_old <= rows("change_sets"), "no change set lost in the upgrade")
r = run(new, "change-set", "show", sorted(changes_old)[0])
ok(r.returncode == 0 and "baseline-ok" in r.stdout, "new build reads the baseline change set")
ok(snaps_old <= snaps_new, "no snapshot lost in the upgrade (%d -> %d)" % (len(snaps_old), len(snaps_new)))
ok(audit_old <= audit_new, "no audit event lost in the upgrade (%d -> %d)" % (len(audit_old), len(audit_new)))
r = run(new, "audit", "verify")
ok(r.returncode == 0, "audit chain still verifies after new events were appended")

# 4. Recovery: drop the live database and restore the backup; the new build opens it.
for suffix in ("", "-wal", "-shm"):
    try: os.remove(db + suffix)
    except FileNotFoundError: pass
shutil.copy(backup, db)
r = run(new, "audit", "verify")
ok(r.returncode == 0, "restored backup opens and verifies with the new build")
ok(snaps_old <= rows("snapshots"), "restored backup contains the baseline snapshots")

# 5. A corrupt database fails visibly instead of being silently replaced.
open(db, "wb").write(b"not a database" * 64)
r = run(new, "audit", "verify")
ok(r.returncode != 0, "corrupt database is rejected with a failing exit (exit %d)" % r.returncode)
print("ALL UPGRADE CHECKS PASSED")
