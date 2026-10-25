# Security and networking

Decide who can reach the application and which files and remote services the backend can access before configuring the listener and HTTPS. Login is required, but administrators can select local paths and internal network addresses. Authentication does not isolate the host or network.

## Choose an entry point

The default `127.0.0.1:8888` listener works for a local browser, SSH forwarding or a reverse proxy on the same host. For direct access on a trusted LAN, you can set:

```toml
listen = "0.0.0.0:8888"
```

This listens on every IPv4 interface, so restrict sources with your firewall. To bind more narrowly, use the server's specific interface IP. The Docker example publishes only on the host's loopback address; the listener inside the container is still on all IPv4 interfaces.

Use an HTTPS reverse proxy for access from untrusted networks, with request timeouts and connection limits at the entry point. The application does not provide a complete set of ingress connection limits. Neither the NixOS module nor `public_url` configures certificates, DNS, a proxy or a firewall.

## Configure one reverse-proxy layer

1. Terminate HTTPS at the proxy and forward application pages, `/api` and media requests. Use a dedicated site root; do not assume arbitrary path prefixes are supported.
2. Restrict backend access to the proxy so clients cannot bypass it and submit forged source headers.
3. Have the proxy write the actual client address in `X-Forwarded-For`. Set `trusted_proxies` only to the proxy IP the backend actually sees.
4. Preserve Range, Content-Range and streaming behavior. Do not cache login or other `no-store` API responses, or buffer task event streams. Remove the `token` query parameter from access logs, or omit query strings entirely.
5. Sign in from a client and check that the logged source address is correct. Then check seeking and task progress.

For a same-host proxy connecting over IPv4 loopback:

```toml
listen = "127.0.0.1:8888"
trusted_proxies = ["127.0.0.1"]
```

This also trusts other processes that can connect over loopback. Use it only when those processes are trusted. If the proxy connects over `::1`, configure that actual address rather than copying the IPv4 example.

`trusted_proxies` accepts individual IPs, not CIDR ranges. An empty list uses the TCP peer address. Only when that peer is trusted does the server use the last entry in `X-Forwarded-For`. Earlier entries do not establish a trusted client identity. This is a single-proxy model; adding every proxy IP does not make a multi-hop chain work correctly.

In Docker, the observed source depends on networking and forwarding mode and is not necessarily the bridge gateway. If direct clients and the proxy appear under the same address, you cannot safely trust arbitrary source headers from that address. Keep the backend port isolated and check logs. The default Compose example deliberately does not trust its gateway.

## What works over plain HTTP

LAN HTTP supports login, browsing, favorites, playback, progress and administration. Service Workers require HTTPS or a browser-recognized secure local context. Ordinary LAN HTTP therefore cannot provide the corresponding PWA installation and offline-opening features. Background playback and media controls also depend on the system and browser; HTTPS alone does not determine their behavior.

HTTP does not encrypt passwords, API tokens, media tokens or content. Network attackers may read or alter them. Use HTTPS across untrusted networks rather than assuming a LAN or shared Wi-Fi is trusted.

## Remote storage and proxy credentials

The backend fetches OpenList and WebDAV media and forwards it to the browser. It does not expose OpenList direct links to the browser. Because OpenList requires an administrator account with base path `/`, protect both those credentials and the backend data directory. WebDAV sends Basic credentials with requests; HTTP does not protect those passwords.

Storage clients use public CA roots compiled into the binary, not custom system CA stores. Mounting a self-signed CA file in a container does not automatically make OpenList or WebDAV trust it. Use a supported certificate chain for HTTPS; do not disable certificate verification. PostgreSQL has a separate certificate configuration path through `sslrootcert`, described in [Configuration](configuration.md#change-databases).

File-managed libraries reference `password_file`. Interface-managed passwords are stored in `secrets/roots/<id>.password` under the data directory. The Nix store is readable by local users, so passwords must not be placed in NixOS `settings`. Backups contain these passwords and need the same access protection.

WebDAV and metadata clients can use HTTP proxy settings from the process environment; the OpenList client does not. For metadata-only proxying, use Network proxy (“网络代理”) in Settings. It overrides the metadata client's environment-based choice without changing WebDAV. The address must be reachable from the backend, not merely from the browser's machine.

## Accounts and access limits

Every signed-in user can browse all libraries. There is no per-user mechanism to hide works or libraries. Grant administrator access only to people trusted to select paths readable by the backend and network addresses it can contact. Local storage does not follow symbolic links, but the service account should still have only the permissions it needs.

API tokens last 30 days and media tokens last 6 hours. A password change or Sign out all devices (“退出所有设备”) immediately revokes that user's existing tokens. Deleting the user invalidates them too. If credentials may have leaked, use these actions and check proxy logs and backups that could contain tokens. See [Accounts](accounts.md).

Login rate limiting counts address failures in the current and preceding minute. At 10 failures it temporarily rejects the address. Misconfigured reverse proxies or shared NAT can make users share this limit. It does not replace strong passwords, HTTPS or entry-point protections.

Remote storage URLs and OpenList direct links can still reach LAN resources. Administrators and remote storage services are part of the trust boundary. For stricter isolation, restrict backend outbound access through a dedicated account, container networking or a firewall. [Restore tests](backup-restore.md#test-a-restore-in-isolation) need their own isolation.
