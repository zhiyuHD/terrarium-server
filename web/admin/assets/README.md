<!-- Owner: src/admin.rs, web/admin/client.css, web/admin/client.js -->

These assets are copied from the Terrarium 0.8.6 console and embedded in the Rust binary. Only the private administration listener serves them.

- `icons.svg` contains the selected, unchanged icon geometry from `lucide-static@1.59.0`, grouped as SVG symbols. Source: https://lucide.dev and https://www.npmjs.com/package/lucide-static/v/1.59.0. `Lucide-LICENSE.txt` retains the package's license and attributions.
- `MiSans-Regular.woff2` and `MiSans-Semibold.woff2` are unmodified files from `MiSans/woff2/` in Xiaomi's https://hyperos.mi.com/font-download/MiSans.zip. They contain the full character sets supplied by Xiaomi.
- `MiSans-LICENSE.pdf` is Xiaomi's license from https://hyperos.mi.com/font/zh/download/. The login page and footer identify MiSans and link to this license.

The administration listener serves all assets with `Cache-Control: no-store`.
