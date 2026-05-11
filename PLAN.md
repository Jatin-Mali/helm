# HELM Execution Plan

## Status Audit (2026-05-12)

| Phase | Feature | Gate | Status |
|-------|---------|------|--------|
| v0.1 | ReAct loop, shell/fs tools, SQLite episodes | cargo test | ✅ DONE |
| v0.2 | Capability model, taint model, audit log | cargo test | ✅ DONE |
| v0.3 | process/service/package/disk/network/logs tools | cargo test | ✅ DONE |
| v0.4 | TUI (ratatui+crossterm) | cargo test | ✅ DONE |
| v0.5 | Browser via PinchTab, injection guard | cargo test | ✅ DONE |
| v0.6 | Skills library, GC, helm skills CLI | cargo test | ✅ DONE |
| v0.7 | Supervisor DAG, FSM, Evidence verifier | cargo test | ✅ DONE |
| v0.8 | install.sh, helm init, docs, release CI | cargo test | ✅ DONE |
| v1.0 | 100-run suite, security, tag v1.0.0 | cargo test | ✅ DONE |
| v1.1 | Git tool, MCP, sessions, snapshots | cargo test | ✅ DONE |
| v1.2 | Graph memory, embeddings, plan cache | cargo test | ✅ DONE |
| v1.3 | Skill learning, model routing, user profile | cargo test | ✅ DONE |
| v1.4 | XDG paths, formal skill format, SkillSuggested wire | cargo test | ✅ DONE |
| v1.5 | NDJSON remote transport, remote event forwarding | cargo test | ✅ DONE |
| v2.0 | Multi-agent, disagreement, parallel sub-agents | see below | 🔄 PLANNED |
| Future | gRPC remote transport, per-host audit, OTel | see roadmap | 🔄 PLANNED |

## v1.5 Release Notes

### Completed in this tranche
- **XDG path compliance**: All `~/.helm/` paths migrated to `$XDG_CONFIG_HOME/helm/`, `$XDG_DATA_HOME/helm/`, `$XDG_CACHE_HOME/helm/` per XDG Base Directory Specification
- **Formal skill format**: `skill.toml` + `SKILL.md` bundle with `Prerequisites`, `SkillMetadata`, and auto-detection in `SkillsManager`
- **Built-in skill bundle**: `nginx-deploy`, `docker-restart`, `git-status` shipped as examples in `builtin_skills/`
- **SkillSuggested wired**: `AgentEvent::SkillSuggested` now emitted on successful episode completion, integrated with `SkillLearner::find_matching_skills`
- **Remote event completeness**: `parse_wire()` in `agent_remote.rs` now handles all 26 `AgentEvent` variants (was 7)

### Remaining roadmap items
- gRPC/protobuf agent-on-remote (current: robust NDJSON-over-SSH)
- Per-host audit files (current: target-partitioned audit rows in shared DB)
- OpenTelemetry exporter
- Full lifecycle hook surface (on_episode, on_iteration, on_step, on_error)

## Execution Steps

1. Run `cargo fmt --check`
2. Run `cargo clippy --workspace --all-targets -- -D warnings`
3. Run `cargo test --workspace --all-targets`
4. Verify `cargo build --release -p helm-cli` succeeds
5. Commit as v1.5 release tranche
