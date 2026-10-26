#!/usr/bin/env bash
# Reproducible acceptance run. Uses temporary state only; never touches real HELM state or services.
#
#   scripts/verify-demo.sh [--strict] [--soak-minutes N]
#
# Required checks FAIL the run. Capabilities whose integration is unavailable on this machine are
# reported UNVERIFIED (never PASS); with --strict (or REQUIRE_ALL=1) an UNVERIFIED capability also
# fails the run, which is what a release gate uses.
#
# Environment:
#   HELM_BIN                path to a release `helm` (default: build target/release/helm)
#   HELM_BASELINE_BIN       build of the pre-repair baseline for the upgrade check
#                           (default: built from git commit fbbbec3 in a temporary worktree)
#   HELM_DEMO_PROVIDER/MODEL real provider for the agent scenario, e.g. ollama / gemma4:31b-cloud
#                           (credentials come from that provider's normal environment)
set -uo pipefail
root=$(cd -- "$(dirname -- "$0")/.." && pwd)
strict=${REQUIRE_ALL:-0}; soak=2
while [ $# -gt 0 ]; do
  case "$1" in
    --strict) strict=1 ;;
    --soak-minutes) soak=$2; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
tmp=$(mktemp -d); trap 'rm -rf -- "$tmp"; [ -z "${worktree:-}" ] || git -C "$root" worktree remove --force "$worktree" >/dev/null 2>&1' EXIT
results=(); failed=0; unverified=0
record() { results+=("$1  $2"); case "$1" in FAIL) failed=1 ;; UNVERIFIED) unverified=1 ;; esac; }
step() { # name, command...
  local name=$1; shift
  echo "=== $name"
  "$@"; local code=$?
  if [ $code -eq 0 ]; then record PASS "$name"; elif [ $code -eq 77 ]; then record UNVERIFIED "$name (integration unavailable)"; else record FAIL "$name (exit $code)"; fi
}

helm=${HELM_BIN:-}
if [ -z "$helm" ]; then
  (cd "$root" && cargo build --release -p helm-cli) || { echo "build failed" >&2; exit 1; }
  helm=$root/target/release/helm
fi
[ -x "$helm" ] || { echo "no helm binary at $helm" >&2; exit 1; }

# --- workspace tests that encode the acceptance scenarios (browser is required here) ---------------
step "agent workflows (scripted provider: approve/deny/resume/cancel)" \
  bash -c "cd '$root' && cargo test -q -p helm-cli --test agent_workflows"
step "cross-frontend outcomes (denied/read-only/dry-run/sandbox/failed verification)" \
  bash -c "cd '$root' && cargo test -q -p helm-cli --test cross_frontend --test execution_modes"
step "agent security (injection, symlinks, redirects, real browser)" \
  bash -c "cd '$root' && HELM_REQUIRE_BROWSER=1 cargo test -q -p helm-agent"
step "dashboard authentication, origin and CSRF" \
  bash -c "cd '$root' && cargo test -q -p helm-cli --test web_security"
step "installer (mismatch, malformed, OS/arch, atomic replace)" bash "$root/scripts/test-install.sh"

# --- operations: collect, explain, review, deny, apply, verify, restart, roll back -----------------
step "operations demonstration" python3 "$root/tests/demo/ops.py" "$helm" "$tmp/ops"

# --- upgrade: state written by the pre-repair build survives ----------------------------------------
baseline=${HELM_BASELINE_BIN:-}
if [ -z "$baseline" ] && git -C "$root" cat-file -e fbbbec3631b23965f692893e0a35b24e43f9f680 2>/dev/null; then
  worktree=$tmp/baseline-src
  git -C "$root" worktree add -q --detach "$worktree" fbbbec3631b23965f692893e0a35b24e43f9f680 \
    && (cd "$worktree" && CARGO_TARGET_DIR="$tmp/baseline-target" cargo build -q -p helm-cli) \
    && baseline=$tmp/baseline-target/debug/helm
fi
if [ -n "$baseline" ] && [ -x "$baseline" ]; then
  step "database upgrade, backup and recovery" python3 "$root/tests/demo/upgrade.py" "$baseline" "$helm" "$tmp/upgrade"
else
  record UNVERIFIED "database upgrade (no baseline build available)"
fi

# --- fleet: two disposable SSH hosts -------------------------------------------------------------------
if command -v docker >/dev/null 2>&1 && docker version >/dev/null 2>&1; then
  step "fleet over two SSH hosts" python3 "$root/tests/demo/fleet.py" "$helm" "$tmp/fleet"
else
  record UNVERIFIED "fleet over two SSH hosts (docker unavailable)"
fi

# --- general agent against a real provider ------------------------------------------------------------
if [ -n "${HELM_DEMO_PROVIDER:-}" ] && [ -n "${HELM_DEMO_MODEL:-}" ]; then
  real() {
    mkdir -p "$tmp/real" && cp -r "$root/tests/demo/project" "$tmp/real/work" || return 1
    XDG_CONFIG_HOME=$tmp/real/config XDG_DATA_HOME=$tmp/real/data XDG_CACHE_HOME=$tmp/real/cache \
      python3 "$root/tests/demo/drive_agent.py" --transcript "$tmp/real/transcript.txt" --timeout 300 \
        --allow 'Write[^\n]*calc\.py|calc\.py' --allow 'Run shell command:\s+(ls|cat|find|head|python3 -m unittest)' -- \
        "$helm" --provider "$HELM_DEMO_PROVIDER" --model "$HELM_DEMO_MODEL" run \
        "Inspect this small project, explain its structure, fix the failing test by correcting the bug, run the tests with python3 -m unittest, and summarize with evidence." \
        --workspace "$tmp/real/work" >/dev/null || return 1
    grep -q "Completed" "$tmp/real/transcript.txt" && (cd "$tmp/real/work" && python3 -m unittest -q 2>&1 | tail -1 | grep -q OK)
  }
  step "general agent, real provider ($HELM_DEMO_PROVIDER/$HELM_DEMO_MODEL): inspect, approved edit, check, summary" real
else
  record UNVERIFIED "general agent with a real provider (set HELM_DEMO_PROVIDER and HELM_DEMO_MODEL)"
fi

# --- prolonged monitoring ---------------------------------------------------------------------------------
step "prolonged monitoring run (${soak} min)" python3 "$root/tests/demo/soak.py" "$helm" "$tmp/soak" --minutes "$soak" --interval 5

echo; echo "================ summary"
printf '%s\n' "${results[@]}"
if [ $failed -ne 0 ]; then echo "RESULT: FAIL"; exit 1; fi
if [ $unverified -ne 0 ]; then
  if [ "$strict" = 1 ]; then echo "RESULT: FAIL (strict: unverified capabilities)"; exit 1; fi
  echo "RESULT: PASS with UNVERIFIED capabilities (not a release claim)"; exit 0
fi
echo "RESULT: PASS"
