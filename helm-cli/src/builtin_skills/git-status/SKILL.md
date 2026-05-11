# Git Status

Shows git repository status with a clean, organized output.

## Prerequisites

- `ShellExec` capability required
- Git repository initialized

## Steps

1. **Check for git repo**:
   ```bash
   git rev-parse --git-dir
   ```

2. **Show status**:
   ```bash
   git status
   ```

3. **Show short status** (optional):
   ```bash
   git status -s
   ```

4. **Show branch info**:
   ```bash
   git branch -v
   ```

5. **Show staged changes**:
   ```bash
   git diff --cached --stat
   ```

## Tool Sequence

1. `shell` — check if in git repo
2. `shell` — git status
3. `shell` — git diff --cached

## Output Format

- Green = staged files
- Yellow = modified files
- Red = untracked files
