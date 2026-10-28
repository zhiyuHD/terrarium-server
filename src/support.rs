//! Donation prompts are public-page UI; dismissal stays entirely in browser storage.
use axum::{http::header, response::IntoResponse};
pub fn page(html: &str) -> String {
    let content = format!(
        "<style>{}</style>{}<script>{}</script>",
        include_str!("../web/support.css"),
        include_str!("../web/support.html"),
        include_str!("../web/support.js")
    );
    if html.contains("</body>") {
        html.replace("</body>", &format!("{content}</body>"))
    } else {
        html.replace("</html>", &format!("{content}</html>"))
    }
}
pub async fn qr() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("../web/support-qr.png").as_slice(),
    )
}
