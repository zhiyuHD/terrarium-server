<!-- Owner: worker.ts, validator.mjs, ../src/legacy_validation.rs -->
# Legacy state validator

Temporary validation fallback for the Rust write preview. `legacy-src/` contains the MIT-licensed state migration and aquarium validators with their transitive imports from `zhiyuHD/cortico-world-terrarium`, source snapshot matching Cortico commit `98dfb72`. The validator retains unknown JSON fields and applies the existing state migrations before Rust checks hashes, timestamps, habitat and score.

`validator.mjs` is the committed Node 22 ESM bundle. Rebuild with `npm install --ignore-scripts && npm run build` in this directory. Rust embeds the bundle and starts one persistent Node worker; there is no runtime dependency on the Cortico checkout. Set `CORTICO_TERRARIUM_LEGACY_NODE` to override the Node executable. Each validation has a five-second timeout; worker failure rejects the report and the next request starts a new worker.

This fallback must remain until Rust validation covers all optional fields, collection specimens, replay frames and legacy migrations with differential tests. Write preview still requires Node 22 or newer. No worker is started in the default read-only mode.
