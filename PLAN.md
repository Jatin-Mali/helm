# HELM Execution Plan

## Status Audit (2026-05-06)

| Phase | Feature | Gate | Status |
|-------|---------|------|--------|
| v0.1 | ReAct loop, shell/fs tools, SQLite episodes | cargo test | ✅ DONE |
| v0.2 | Capability model, taint model, audit log | cargo test | ✅ DONE |
| v0.3 | process/service/package/disk/network/logs tools | cargo test | ✅ DONE |
| v0.4 | TUI (ratatui+crossterm) | cargo test | ✅ DONE |
| v0.5 | Browser via PinchTab, injection guard | cargo test | ✅ DONE |
| v0.6 | Skills library, GC, helm skills CLI | cargo test | ✅ DONE |
| v0.7 | Supervisor DAG, FSM, Evidence verifier | cargo test | ✅ DONE |
| v0.8 | install.sh, helm init, docs, release CI | manual | ✅ DONE |
| v1.0 RC | 100-run suite, security, tag v1.0.0-rc1 | see below | 🔄 IN PROGRESS |

## v1.0 RC Checklist

- [x] cargo fmt --check
- [x] cargo clippy --workspace --all-targets -- -D warnings
- [x] cargo test --workspace --all-targets (265 passing)
- [x] 25-run deterministic reliability suite
- [ ] 100-run release suite (currently #[ignore])
- [x] browser prompt injection security test
- [x] release.yml CI workflow
- [x] docs: providers.md, threat-model.md, troubleshooting.md
- [ ] Tag v1.0.0-rc1

## Execution Steps

1. Run 100-run suite: `cargo test deterministic_100_run -- --ignored`
2. Check e2e_react tests
3. Verify all tests still clean
4. Commit any final fixes
5. `git tag v1.0.0-rc1`
