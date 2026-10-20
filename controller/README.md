<!-- Owner: admin.mjs, release.mjs, runner.mjs -->
# Atomic release controller

These MIT-licensed Node 22 bundles come from `cortico-world-terrarium` at Cortico commit `98dfb72`. They retain the schema 2 Native executable inventory, checksums, pinned controller, startup checks and same-data-epoch rollback. The Native server starts without arguments under `runner.mjs`.

`../scripts/pack-release.mjs` includes these files and the public/private assets in each release. They contain no deployment data. An existing deployment must pin this Native-capable controller before using a Native payload. Cross-epoch transitions use `../scripts/migrate-storage.mjs`; the normal controller refuses them.
