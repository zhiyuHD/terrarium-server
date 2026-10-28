<!-- Owner: client.js, build.mjs -->
# Forum Markdown

`render(body)` returns the public forum's Markdown content element. Raw HTML is escaped. Links allow HTTP, HTTPS and mailto; images allow HTTP/HTTPS, load lazily and omit the referrer. External links open with noopener/noreferrer. The forum title remains plain text.

Install the pinned dependencies with `npm ci`, then run `npm run build`. The generated IIFE is embedded in the Native binary and included before the forum module. Dependency licenses are retained in `../web/forum-markdown.LICENSE.txt`.
