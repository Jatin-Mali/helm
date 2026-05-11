# Deploy NGINX

Deploys and configures a NGINX web server.

## Prerequisites

- `ShellExec` capability required
- `SystemService` capability required
- Root/sudo access on the target system

## Steps

1. **Install NGINX** using the package tool or shell:
   ```bash
   sudo apt-get install -y nginx  # Debian/Ubuntu
   sudo dnf install -y nginx      # Fedora/RHEL
   ```

2. **Configure NGINX** — edit `/etc/nginx/nginx.conf` or add sites to `/etc/nginx/conf.d/`

3. **Validate config**:
   ```bash
   sudo nginx -t
   ```

4. **Start/enable service**:
   ```bash
   sudo systemctl enable --now nginx
   ```

5. **Verify**:
   ```bash
   sudo systemctl status nginx
   curl -I http://localhost
   ```

## Tool Sequence

1. `shell` — install nginx package
2. `shell` — configure nginx
3. `shell` — validate config
4. `service` — start/enable nginx
5. `shell` — verify deployment
