# Running Livrarr on Proxmox (LXC)

The supported path is **Docker-in-LXC**: run a small Debian/Ubuntu LXC container, install Docker inside it, and run the Livrarr image there. (A native, no-Docker install is not provided yet — Livrarr ships as a Docker image only.)

## 1. Create the LXC

A **privileged** LXC is the simplest. If you use an **unprivileged** LXC (recommended by Proxmox for isolation), it needs nesting and keyctl enabled so Docker can run inside it. In the LXC's Options → Features, enable:

- **nesting** = 1
- **keyctl** = 1

Or in `/etc/pve/lxc/<vmid>.conf`:

```
features: nesting=1,keyctl=1
```

Give it a couple of cores, ~1 GB RAM, and enough disk for your database and covers (the media library lives on a mount, below).

## 2. Install Docker in the LXC

```bash
apt update && apt install -y ca-certificates curl
curl -fsSL https://get.docker.com | sh
```

## 3. Run Livrarr

Copy the project's `docker-compose.yml` into the LXC, edit the volume paths and `PUID`/`PGID`, then:

```bash
mkdir -p config && chown -R 1000:1000 config
docker compose up -d
```

Open `http://<lxc-ip>:8789`.

> **Unprivileged LXC — use host networking.** Publishing ports (`ports:` / `-p`) **fails** inside an unprivileged LXC: Docker needs the `net.ipv4.ip_unprivileged_port_start` sysctl, which the container's user namespace blocks (`error during container init: … ip_unprivileged_port_start … permission denied`). Fix: in the compose service, remove the `ports:` block and add `network_mode: host` — Livrarr then listens on the LXC's `:8789` directly. A **privileged** LXC does not hit this. Verified on Proxmox VE 9 (unprivileged LXC, nesting+keyctl): with `network_mode: host`, the PUID/PGID root-start-drop works and `/config` ownership (incl. repair of pre-existing files) is fixed correctly.

## ⚠️ Ownership on **unprivileged** LXC (read this)

An unprivileged LXC **shifts user IDs**: uid/gid inside the container are offset by a large number on the Proxmox host (typically +100000, so container `1000` = host `101000`). This affects any host directory you bind-mount into the LXC for your library/downloads:

- The files must be owned by the **host-side** shifted id (e.g. `101000:101000`), or
- Add an id-mapping (`lxc.idmap`) so container `1000` maps to the host uid that already owns your media.

Inside Docker, `PUID`/`PGID` refer to the **LXC's** namespace — Livrarr fixes ownership of `/config` there, but it cannot fix a mismatch introduced by the LXC↔host id shift. Get the LXC bind-mount ownership right on the Proxmox host first.

Privileged LXCs do not have this shift and are simpler if you don't need the extra isolation.
