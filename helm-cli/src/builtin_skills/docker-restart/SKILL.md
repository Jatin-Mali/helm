# Restart Docker

Safely restarts the Docker daemon.

## Prerequisites

- `ShellExec` capability required
- `SystemService` capability required
- Appropriate permissions for docker group

## Steps

1. **Check running containers**:
   ```bash
   docker ps --format '{{.Names}}'
   ```

2. **Stop non-essential containers** if needed (optional)

3. **Restart Docker**:
   ```bash
   sudo systemctl restart docker
   ```

4. **Wait for Docker to be ready**:
   ```bash
   sleep 3 && docker info
   ```

5. **Verify containers** restarted if auto-restart is configured

## Tool Sequence

1. `shell` — list running containers
2. `service` — restart docker service
3. `shell` — verify docker is running
