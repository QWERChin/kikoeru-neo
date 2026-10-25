# Install on NixOS

The NixOS module creates a service account, data directory and systemd service, and installs the account-management and backup commands. You need an `x86_64-linux` or `aarch64-linux` NixOS host and permission to edit and activate its system configuration.

## Add the module

Add this input to your existing system flake:

```nix
kikoeru-neo.url = "github:QWERChin/kikoeru-neo";
```

Accept `kikoeru-neo` in the arguments to `outputs`, then add these entries to the target system's `modules` list. Keep this input's own nixpkgs; do not add an `inputs.nixpkgs.follows` override.

```nix
kikoeru-neo.nixosModules.default
{
  services.kikoeru-neo = {
    enable = true;
    settings.listen = "127.0.0.1:8888";
  };
}
```

Keep `flake.lock` to retain the selected source version. This configuration uses SQLite at `/var/lib/kikoeru-neo` and runs as user and group `kikoeru-neo`. See [Configuration](configuration.md#nixos-module-options) for module options.

## Activate, create an administrator and connect

From your system configuration directory, activate your existing target. For a target named `media`:

```sh
sudo nixos-rebuild switch --flake .#media
systemctl status kikoeru-neo
sudo -u kikoeru-neo kikoeru-neo user create --admin admin
```

Activate before creating the account: activation creates the system user and installs the configured CLI wrapper. Enter a password of at least 8 characters twice. Success prints `created admin admin`.

Open `http://127.0.0.1:8888` on the server. If it has no browser, forward the port from your own computer, replacing `media.example.lan` with your server's address:

```sh
ssh -L 8888:127.0.0.1:8888 media.example.lan
```

Keep the SSH connection open and visit the same loopback URL on your computer. Choose another local port if 8888 is occupied. After signing in, follow [Libraries and scanning](libraries.md) to add media and play a track.

## File access and configuration

For local libraries, grant the service user directory traversal and file-read access through an existing media group or ACL. Keep the application data directory private to the service user. The module's systemd restrictions make the media filesystem read-only to the service while allowing writes to its data directory.

The module writes `settings` into the Nix store, which local users can read. Do not put passwords there. Passwords entered through the interface are stored in the data directory. Declarative remote libraries use `password_file`; supply those files through your own secret-management mechanism. See [Libraries](libraries.md#declare-libraries-in-a-file).

The installed `kikoeru-neo` command defaults to the service's configuration and data directory. Run account and backup commands as the service user. Changing `--data-dir` does not remove a configured PostgreSQL address. Use an [isolated configuration](backup-restore.md#test-a-restore-in-isolation) when working with a restored copy.

## PostgreSQL

For a new installation, you can enable:

```nix
services.kikoeru-neo.database.createLocally = true;
```

The module enables local PostgreSQL and creates a role and database named after the service user. It connects through `/run/postgresql` using peer authentication, with the port taken from the PostgreSQL configuration. To move existing SQLite data, first [back up, stop the service and copy the database](configuration.md#change-databases). Enabling the option alone does not move the data.

## Check the service

```sh
journalctl -u kikoeru-neo -n 100 --no-pager
journalctl -u kikoeru-neo -f
```

The service should be running, and `http://127.0.0.1:8888/api/ready` should return HTTP 200. Readiness succeeds even before an administrator exists. Check storage access separately after signing in.

The module does not configure a firewall, DNS, certificates or a reverse proxy. For access from other machines, see [Security and networking](security.md). Activating changed service configuration restarts the service, so check for scans first as described in [Upgrades](upgrade.md).
