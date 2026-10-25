#!/usr/bin/env bash
# Disposable two-host SSH fleet in Docker: `fleet.sh up <state-dir> <helm-binary>` / `fleet.sh down`.
# Containers are named helm-demo-a / helm-demo-b; nothing else on the Docker host is touched.
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
names=(helm-demo-a helm-demo-b)
cmd=${1:?up|down}
case "$cmd" in
down)
  docker rm -f "${names[@]}" >/dev/null 2>&1 || true
  ;;
up)
  state=${2:?state dir}; helm=${3:?helm binary}
  mkdir -p "$state"
  [ -f "$state/id_ed25519" ] || ssh-keygen -q -t ed25519 -N '' -f "$state/id_ed25519"
  docker image inspect helm-demo-sshd >/dev/null 2>&1 || docker build -q -t helm-demo-sshd "$here/fleet" >/dev/null
  docker rm -f "${names[@]}" >/dev/null 2>&1 || true
  for n in "${names[@]}"; do
    docker run -d --name "$n" --hostname "$n" helm-demo-sshd >/dev/null
    docker cp "$state/id_ed25519.pub" "$n:/root/.ssh/authorized_keys"
    docker exec "$n" chown root:root /root/.ssh/authorized_keys
    docker exec "$n" chmod 600 /root/.ssh/authorized_keys
    docker cp "$helm" "$n:/usr/local/bin/helmops"
    docker exec "$n" ln -sf helmops /usr/local/bin/helm
    # Distinct machine identity per host (containers share the kernel's boot id).
    docker exec "$n" sh -c 'cat /proc/sys/kernel/random/uuid | tr -d - > /etc/machine-id'
    docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$n" > "$state/$n.ip"
  done
  ;;
esac
