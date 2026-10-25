# Upgrade or return to an older version

Choose a target version before upgrading, check for database changes, and keep a complete backup with the corresponding old software. Startup applies database migrations automatically. An older program may refuse a newer schema, so replacing the binary alone is not a complete rollback.

## Prepare

1. Save the current version and deployment configuration. Open the service's `/api/version` in a browser to record its version and available build information.
2. Select the target version, read its change notes and check whether its database migrations differ. If that information is insufficient, assume migration is possible and [back up](backup-restore.md) first.
3. Check the Settings task list for running or queued scans. Stop submitting new tasks and temporarily disable automatic scans and directory watching if needed.

A restart interrupts running scans. They are marked `interrupted` on startup and need a new scan request. Queued tasks are requeued rather than canceled. Already committed data remains, but this is not a guarantee of automatic resume from the interruption point.

## NixOS or a standalone binary

NixOS pins the input through `flake.lock`. Save the updated lock file and activate the system from that configuration. For a fixed release, point the input at an existing version tag or revision. Keep later deployments from reusing an older lock file unintentionally.

From your system flake directory, for a target named `media`:

```sh
nix flake update kikoeru-neo
sudo nixos-rebuild switch --flake .#media
systemctl status kikoeru-neo
journalctl -u kikoeru-neo -n 100 --no-pager
```

`nix flake update` selects the newest revision allowed by the input URL, not a release chosen for your installation. Change that URL first when you need an exact version. Do not run the upgrade commands before selecting the target and completing the backup.

For a standalone binary, check out the selected source version and run `nix build .#neo-server`. Stop the old process, then start the new package with the same configuration and data directory. Keep the old package or pinned inputs that can rebuild it.

## Docker

Use a full version tag or image digest, not a floating series tag as your rollback reference. Rebuild and load source-built images using the [Docker guide](install-docker.md). Only use `docker compose pull kikoeru-neo` for a GHCR image after confirming the selected release is accessible.

After updating `image` in Compose and completing the backup:

```sh
docker compose up -d
docker compose ps
docker compose logs --tail 100 kikoeru-neo
```

Different local builds can share a source version number. Keep the old image ID or a separate tag instead of relying on an overwritten tag for recovery. Upgrading does not require deleting the data volume.

## Verify the upgrade

`/api/ready` should return HTTP 200 and `/api/version` should show the intended version. Sign in and check storage connectivity, covers, playback and saved progress. Readiness does not probe every library. Resubmit interrupted scans after confirming storage access.

The desktop application has no automatic updater. Obtain the new package as described in [Desktop application](desktop.md), quit the old application completely, then replace it. Server mode loads the server's frontend, so check compatibility when upgrading either the server or the desktop shell.

## Return to an older version

Stop the new service and preserve its failed state. Restore the backup to an empty directory or database, then start the matching software and configuration. If the old program rejects the new schema, do not edit migration records or delete tables to bypass the check. Follow [Backup and restore](backup-restore.md).

Restoring loses changes made after the backup. Keep the failed copy for comparison, but do not run two instances against the same data directory or database.
