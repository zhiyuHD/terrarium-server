<!-- Owner: render.mjs, build.mjs -->
# Photo renderer

The native server submits a verified observation snapshot over stdin and receives one PNG on stdout. The worker imports the same `web/scene.js` used by the public gallery, with a single animation frame for water. Rendering uses MiSans and `@napi-rs/canvas` without Chromium or an LLM.

Run `npm ci`, `npm run build`, then `npm test` here. The build creates `dist/photo-worker.mjs`, the matching Linux GNU Canvas binding, fonts and licenses. The server release packer includes those files as checked release assets. Build on the Linux architecture used by the target server.

Each worker handles one photo and exits. The server limits render concurrency, duration and response size. Finished PNGs are stored as Core image attachments; this process maintains no photo database.
