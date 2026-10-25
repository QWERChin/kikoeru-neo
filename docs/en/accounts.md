# Manage accounts

The server has no anonymous access or web registration. Create and delete accounts, including administrators, through the local CLI. Users can change their own passwords or sign out all devices through the interface. First check that the command uses the running service's configuration and database.

## Create the first administrator

After activating the NixOS module:

```sh
sudo -u kikoeru-neo kikoeru-neo user create --admin admin
```

With the Compose configuration from [Docker installation](install-docker.md), run this in the deployment directory:

```sh
docker compose run --rm kikoeru-neo user create --admin admin
```

For a standalone binary, select the same configuration and data directory as the service:

```sh
KIKOERU_CONFIG=/etc/kikoeru-neo/config.toml \
  kikoeru-neo --data-dir /srv/kikoeru-neo user create --admin admin
```

Enter the password twice. Success prints `created admin admin`; you can then sign in. Passwords must have at least 8 characters. Usernames must have 1 to 64 characters, with no control characters or leading or trailing whitespace. Choose a long password that you do not reuse elsewhere.

## Add, reset and delete accounts

Use the command prefix appropriate to your installation above, followed by one of these subcommands. `reader` and `operator` are example account names.

| Subcommand | Result |
|---|---|
| `user create reader` | Creates an ordinary user and prompts for a password. |
| `user create --admin operator` | Creates another administrator. |
| `user set-password reader` | Resets the password and revokes all existing tokens for that user. |
| `user delete reader` | Deletes the user and invalidates their tokens. The last administrator cannot be deleted. |

There is no command or interface for changing an existing user's administrator role. Creating an administrator with an existing username is rejected, not treated as a role change.

For scripts, `create` and `set-password` accept `--password-stdin`, which reads the first line from standard input. For example, with an existing `admin.pw` file readable only by you:

```sh
docker compose run --rm -T kikoeru-neo user set-password admin --password-stdin < admin.pw
```

After success, retain or remove temporary password files according to your credential-storage policy. Do not put passwords in command arguments, shell history or shared logs.

## Change a password or sign out in the interface

The account section of Settings (“设置”) lets users change their password after entering the current one. A successful change invalidates existing logins and requires signing in again. Sign out all devices (“退出所有设备”) also revokes the user's API and media tokens. Signing out only the current device clears that page's saved login without revoking other tokens.

Ordinary users can browse and play all libraries; favorites, reviews and progress are personal. Administrators can also configure libraries, run scans and edit metadata. There is no per-user library access control.

## Imported accounts and initialization problems

Legacy kikoeru-express passwords are not imported. Set new passwords with `user set-password` for accounts that need access. A merge creates missing users without passwords and without administrator privileges; existing users keep their roles. See [Migration](migration.md).

A successful `/api/ready` response does not mean an administrator exists. If the login page says initialization is required, create an administrator. If creation succeeds but the page still says this, compare configuration files, data directories and PostgreSQL URLs as described in [Troubleshooting](troubleshooting.md).

Desktop local mode creates its own local administrator and signs in automatically. It does not need these server initialization steps. See [Desktop application](desktop.md).
