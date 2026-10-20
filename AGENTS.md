# AGENTS.md

- Production currently runs the TypeScript implementation in cortico-world-terrarium. Do not deploy this preview as a replacement until write protocols, administration and data migration pass compatibility tests.
- Keep tests offline. Use temporary SQLite files and synthetic ecosystem data; never commit deployment data, tokens, feedback or announcements.
- Run cargo fmt --check, cargo test --locked and cargo clippy --all-targets --locked -- -D warnings before commits.
- Keep the TS wire protocol, IDs, tokens and announcement cursors compatible. A storage migration requires a new data epoch and a tested export path before production cutover.
- Keep blocking SQLite work outside Tokio executor threads. Confirm writes only after the database transaction commits.
- A release controller supports a Native binary only after its pinned controller is upgraded. Do not overwrite an active release payload.
