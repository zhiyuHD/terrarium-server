<!-- Owner: ../src/admin_views.rs, ../src/admin.rs, ../src/operations.rs, ../web/admin/client.js, ../web/admin/index.html -->
# Administration lists

The private listener and authentication rules apply to every view. `GET /api/admin/boxes` without a view retains its full-list response for existing administration clients. New views change response size without changing stored snapshots, credentials, policies or announcement targets.

- `boxes?view=page&page=1&pageSize=50&q=...&version=...` filters the entire registry, sorts by report time descending with ID as tie-breaker, and returns only the requested page. The response includes `total`, `filtered`, `page`, `pages`, `pageSize`, global version choices and dashboard counts. Page size defaults to 50 and is limited to 1–100; pages beyond the end clamp to the last page. Search is case-insensitive across name and ID. Missing versions retain the inferred legacy or unreported labels.
- `boxes?view=statistics&scope=reported|recent|online` returns global aggregate charts, independent of the box page and filters. Unreported identities do not enter charts. Recent reports are within the preceding 24 hours; online means a current process-local WS connection.
- `boxes?view=lookup&ids=<JSON array>` resolves up to 100 IDs for names and registration state in feedback and recipient previews. Responses contain no credentials or simulation state.
- `boxes?view=ids&q=...&version=...` resolves registered recipients across all matching pages. More than 100 registered matches returns `tooMany: true` and an empty list, matching the existing targeted-announcement limit.
- `feedback?view=page&page=1&pageSize=50&pendingOnly=true` filters before pagination and returns newest IDs first. Legacy `feedback?after=<cursor>` remains unchanged.

The browser loads only the active section's detail. Initial login does not download feedback history, notices, controls or statistics charts. Box and feedback DOM lists are bounded to 50 rows or cards; selection survives paging and filtering, with a limit of 100 recipients. Selection updates the clicked row rather than rebuilding the table. Recipient names use an ID map, date formatting reuses one formatter, and search waits 250 ms before requesting its latest value. Responses for an obsolete query or session are discarded. Refreshes retain the current page and filter. Statistics always aggregate all qualifying reports rather than the displayed page.

Registry reads still use the compact summary table; pagination bounds serialization, network payload and browser work, not the metadata scan itself. JSON writes and simulation snapshots are unaffected. A rollback to a server without these optional views requires reloading the management page.
