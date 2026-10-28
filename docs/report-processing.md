<!-- Owner: ../src/summary_cache.rs, ../src/ingestion.rs, ../src/protocol.rs, ../src/metrics.rs, ../src/http.rs -->
# Report processing

Reports retain four active processing slots, a two-second admission deadline and a shared 24 MiB raw-buffer budget. HTTP reserves received bytes rather than reserving its eight-MiB maximum for every request. WebSocket bounds and legacy HTTP policy responses remain compatible. Two persistent Node workers provide the temporary legacy-validation fallback; cancelling a request still disposes its worker before another request can read it.

CPU-intensive parsing, delta application and checksum verification run outside the Tokio executor. Canonical JSON writes to a buffered SHA-256 sink and a size-counting sink, preserving ECMAScript numbers, UTF-16 key sorting and integer-index ordering. Acknowledgments reuse an already verified client checksum. Validated state and committed entry ownership move between stages without cloning full histories.

`box_summaries` is a derived SQLite cache containing score parts, names, wallet values and client metadata. Each snapshot transaction updates its cache row. Triggers invalidate rows on box updates or deletions, including writes by older binaries. Startup rebuilds missing rows from snapshots without changing IDs, credentials, stored snapshots or announcement cursors. Backups include the cache; JSON exports exclude it and imports rebuild it. This optional cache keeps data epoch 2 and SQLite user_version 1; older releases ignore it.

Ecology rankings use indexed top-row reads and indexed own-rank counts. Coin rankings and private box statistics read the compact cache. Two concurrent leaderboard reads limit blocking-thread contention; response encoding also runs on the blocking pool. Legacy full-state leaderboard responses retain their shape.

`GET /health` includes aggregate reportProcessing counters: active/waiting requests, admission/buffer/connection-limit rejections, accepted reports and cumulative mean/max waiting and processing times. Counters reset on restart. Only committed reports count as accepted; busy replies do not acknowledge a save.
