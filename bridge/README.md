<!-- Owner: worker.ts, validator.mjs, ../src/legacy_validation.rs -->
# Legacy state validator

Temporary validation fallback for the Rust write service. `legacy-src/` contains the MIT-licensed state migration and aquarium validators with their transitive imports from `zhiyuHD/cortico-world-terrarium`, following the World land and water state schemas. The validator retains unknown JSON fields and applies the existing state migrations before Rust checks hashes, timestamps, habitat and score.

`validator.mjs` is the committed Node 22 ESM bundle. Rebuild with `npm install --ignore-scripts && npm run build` in this directory. Rust embeds the bundle and starts one persistent Node worker; there is no runtime dependency on the Cortico checkout. Set `CORTICO_TERRARIUM_LEGACY_NODE` to override the Node executable. Each validation has a five-second timeout; worker failure rejects the report and the next request starts a new worker.

This fallback must remain until Rust validation covers all optional fields, collection specimens, replay frames and legacy migrations with differential tests. The write service still requires Node 22 or newer. Startup probes the worker before opening a listener. No worker is started in the default read-only mode.

校验器兼容 0.8.7 第一阶段的 sunflower、mint、ladybug、葵花籽食物和持久落籽轮次，以及可选的叶片采食损伤（leafDamage）；旧存档、历史与收藏仍按原协议校验，无存储迁移。
