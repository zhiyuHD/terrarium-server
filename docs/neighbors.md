<!-- Owner: src/neighbors.rs, src/ingestion.rs, src/policy.rs, src/store.rs -->
# Neighbor visits and report admission

`POST /api/neighbors` authenticates a land box with a Bearer token and body `id`. Actions are status, configure (enabled), visit (target, note), gift (target, species), harvest (target, plant) and ack (cursor). Both boxes must opt in before social writes. Five actions per UTC day, 32 mailbox entries and 32 recent visit records per box bound delivery. Seed packets are free welcome gifts; shared harvest grants one separate visitor fruit per crop round and never edits an owner's submitted state. Harvest requires a snapshot from the last 24 hours. Clients save received inventory and cursor before acknowledgment.

The optional `neighbors.json` document uses the existing epoch-2 legacy_documents table. Backups include it and current exports/imports retain it. Older binaries ignore the document; no existing box, credential, snapshot or cursor format changes.

Report admission retains four concurrent processing slots and waits at most two seconds for a slot. Raw incoming report buffers remain globally bounded at 24 MiB. Admission timeout returns a retryable 503. Supported-client policy checks project one client's metadata rather than deserialize all clients. Policy cache entries expire within 60 seconds and compare current effective cloud controls before reuse, including scheduled changes. Announcement polling skips writer transactions when no receipts need cleanup.

Gallery ranking reads scalar metadata before selecting a page; full snapshots are loaded only for that page. The counts index is derived metadata and does not change the data epoch.
