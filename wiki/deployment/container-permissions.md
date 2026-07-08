# Container Permissions (PUID / PGID)

How Livrarr's Docker image handles the user it runs as. Implemented for #158 (+ #105 Unraid, #106 Proxmox). Pure packaging — no Rust code involved.

## Model — root-start, drop-privileges (LinuxServer.io convention)

The runtime image has **no `USER` directive**, so it starts as root. `docker/entrypoint.sh` then:

1. **Argument handling** — defaults the app invocation only when no command was given; passes through user flags; runs a raw command as-is (debug escape hatch). Never discards args.
2. **Validates PUID/PGID up front** (before the mode split, so invalid values are rejected in every mode). Rejects non-numeric and **any leading-zero form** — `su-exec`/`find` read `00` as uid 0, so a string-exact `"0"` check is insufficient. Empty → `1000` (friendly for `PUID=${PUID}` with an unset `.env`).
3. **If already non-root** (`user:`/`--user`, or a rootless engine) → exec the app as-is. Nothing privileged.
4. **If root** → `find`-repair ownership of `/config` only (mis-owned entries, `chown -h` so a symlink can't redirect outside `/config`), preflight the drop (`su-exec … true`) for a clear error, then `exec su-exec PUID:PGID tini -- app`.

**Invariants:** app never runs as root; only `/config` ownership is touched (never the library); after handoff PID 1 is a non-root `tini`; no `/etc/passwd` writes (numeric su-exec → read-only-rootfs safe).

## Capabilities

Root-start needs a few of the caps that `cap_drop: ALL` strips back. The default compose adds the minimal set:

```yaml
cap_drop: [ ALL ]
cap_add:  [ CHOWN, SETUID, SETGID, DAC_OVERRIDE ]   # DAC_OVERRIDE handles mode-000 host dirs
security_opt: [ no-new-privileges:true ]            # compatible — only blocks *gaining* privs
```

`FOWNER` not needed (never `chmod`s). `no-new-privileges` does **not** block dropping root.

## Modes

- **Default (S1):** root-start → fix `/config` → drop to PUID:PGID. Works out of the box like every *arr.
- **Hardened / rootless (S2):** `user: "1000:1000"` + `cap_drop: ALL` (no cap_add). Never root; PUID/PGID ignored; pre-`chown` host `./config`. **Required** for rootless Docker/Podman (they report uid 0 inside a userns without real privilege).
- **Misconfigured (S3):** root + caps stripped → **fail loud** with a fix-it message (never silently runs as root).

## Platform gotchas

- **Unraid** ignores compose `cap_add` — declare caps in the CA template's `ExtraParams` (`docker/unraid-template.xml`). Unraid default is PUID=99/PGID=100.
- **Proxmox unprivileged LXC** shifts uids (+100000 on the host). Livrarr fixes `/config` inside the LXC namespace but cannot fix the LXC↔host id shift — get bind-mount ownership right on the host; needs `nesting=1`, `keyctl=1`. See `docs/proxmox-lxc.md`.

## Upgrade impact

README Quick-Start users (no `cap_drop`) are unaffected. Users of the hardened `docker-compose.yml` (`cap_drop: ALL`, no `cap_add`) hit S3 fail-loud until they add `cap_add` or switch to `user:`. Fail-loud is deliberate — without SETUID a root process can't drop, so "graceful fallback" would mean silently running as root.

## Verification

Entrypoint logic: 16/16 in `alpine:3.21` (0/00/000/01000 reject, argv passthrough, subtree repair, S1/S2/S3). Real image: 6/6 end-to-end (health 200 as uid 99, `/config` auto-owned, hardened mode works, misconfig fails loud).
