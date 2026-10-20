<!-- Owner: ../src/main.rs, ../src/assets.rs, ../scripts/pack-release.mjs, ../scripts/migrate-storage.mjs, ../controller/runner.mjs -->
# Native deployment

The Native server requires Linux x64 or arm64, a compatible libc and Node 22+ for the temporary legacy state validator. Build on the target architecture. `pack-release.mjs` produces a schema 2 release with the executable, public/private assets, their licenses and the Node release controller. Its inventory and hashes are checked before installation. Payload files are sealed after installation.

The pinned controller must support schema 2 Native releases before activation. It launches `server` without arguments. Managed startup requires `CORTICO_TERRARIUM_DEPLOY_ROOT` and a valid `CORTICO_TERRARIUM_RELEASE_ID`, binds the public listener to `127.0.0.1:${PORT:-8787}` and enables writes. The database defaults to `<root>/data/terrarium.sqlite`; `CORTICO_TERRARIUM_DATABASE` overrides it. Existing `CORTICO_TERRARIUM_ADMIN_HOST`, `CORTICO_TERRARIUM_ADMIN_PORT` and `CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR` settings select the private listener. CLI `serve` retains its explicit `--enable-writes` switch.

Managed pages pin scripts, styles and fonts to their release ID. Historical assets come from installed `versions/<id>/`, using separate public and private allowlists. Keep old release directories while pages may still reference their assets.

## JSON to SQLite

JSON payloads use data epoch 1; SQLite payloads use epoch 2. The normal release controller rejects activation and rollback across these epochs. The migration command requires an installed epoch 2 Native release, an epoch 1 current release, no pending activation and no existing `data/terrarium.sqlite`. It does not replace an existing database. Run it on the deployment host as the service owner. The user service defaults to `cortico-box.service`; `--service` and `--health-url` select an isolated rehearsal deployment.

Without `--apply` the command checks prerequisites and prints its intended operation. With `--apply` it holds the controller's `flock`, installs an `ExecStartPre` migration check and writes a durable journal at `.deployment/storage-migration.json`. It stops the user service, copies the JSON documents into a private backup directory, imports SQLite, checks integrity and verifies that the export preserves the input documents. IDs, tokens, feedback, announcements, controls and protocol notice cursors remain unchanged. Original JSON files remain on disk.

Only after import and export verification does the command select the Native release and restart the service. WS clients reconnect and submit a complete baseline. The service address and client credentials stay the same. The migration clears the normal `previous` release because an epoch 1 binary cannot read epoch 2 writes; subsequent epoch 2 updates can use normal atomic activation and rollback.

The journal blocks service startup during import or rollback. A process interruption or reboot cannot automatically start a writer against a partially migrated dataset. `rollback --apply` recovers recorded unfinished migrations. Inspect the journal and service logs before recovery. Do not delete the journal or bypass `ExecStartPre` while migration is incomplete.

## Storage rollback

`rollback --apply` uses the migration journal and requires that the current release is still the recorded Native target or original TS release. It stops the writer and exports the latest committed SQLite data to a new directory. It then atomically replaces the individual JSON files, restores the TS release state, archives the SQLite files in the migration backup directory and starts TS. Feedback, registrations, notices and reports accepted after the cutover are included. If the current release has since changed, first return to the recorded epoch 2 target with the normal controller.

A failed Native startup invokes this export-based rollback automatically. If export or rollback cannot preserve committed data, the service remains stopped and the journal retains the recovery state. Never restore only the pre-cutover JSON snapshot after the Native service has accepted writes.

Snapshots, exports and databases contain secrets and user data. Backup directories use mode 0700 and files use 0600; do not copy them into the source repository. Keep the migration backup until the retention policy permits deletion.
