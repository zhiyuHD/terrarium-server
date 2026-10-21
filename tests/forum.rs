use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::collections::HashMap;
use tempfile::TempDir;
use terrarium_server::{
    forum,
    http::{App, router},
    ingestion::Ingestion,
    operations::OperationError,
    store::Store,
};
use tower::ServiceExt;
fn fixture() -> (TempDir, Store, String, String) {
    let dir = TempDir::new().unwrap();
    let s = Store::open(dir.path().join("data.sqlite")).unwrap();
    let a = s.register(Some("garden-a")).unwrap().unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = s.register(Some("garden-b")).unwrap().unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (dir, s, a, b)
}
fn run(
    s: &Store,
    method: Method,
    path: &str,
    query: &[(&str, &str)],
    input: Value,
    token: &str,
    now: i64,
) -> anyhow::Result<Value> {
    forum::execute(
        s,
        &method,
        path,
        &query
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<HashMap<_, _>>(),
        &input,
        token,
        now,
    )
}
fn err(result: anyhow::Result<Value>, code: u16) {
    assert_eq!(
        result
            .unwrap_err()
            .downcast::<OperationError>()
            .unwrap()
            .status,
        code
    )
}
fn body(author: &str, body: &str) -> Value {
    json!({"id":author,"title":"薄荷养育心得","body":body,"clientVersion":"0.8.7"})
}
#[test]
fn cloud_schedule_and_logo_survive_restart_and_close_all_forum_operations() {
    let (dir, s, a, _) = fixture();
    err(
        run(&s, Method::GET, "threads", &[], Value::Null, "", 0),
        503,
    );
    assert!(!s.forum_path().exists());
    assert_eq!(s.controls(0).unwrap()["logo"]["version"], "0.8.6");
    s.update_named_control(
        "forum",
        &json!({"enabled":true,"at":"2026-10-22T00:00:00+08:00"}),
        0,
    )
    .unwrap();
    s.update_named_control(
        "logo",
        &json!({"version":"0.8.7","at":"2026-10-22T00:00:00+08:00"}),
        0,
    )
    .unwrap();
    let at = chrono::DateTime::parse_from_rfc3339("2026-10-22T00:00:00+08:00")
        .unwrap()
        .timestamp_millis();
    err(
        run(
            &s,
            Method::POST,
            "threads",
            &[],
            body("garden-a", "未开放"),
            &a,
            at - 1,
        ),
        503,
    );
    drop(s);
    let s = Store::existing(dir.path().join("data.sqlite")).unwrap();
    assert_eq!(s.controls(at - 1).unwrap()["logo"]["version"], "0.8.6");
    assert_eq!(s.controls(at).unwrap()["logo"]["version"], "0.8.7");
    let thread = run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "薄荷需要适宜光照"),
        &a,
        at,
    )
    .unwrap();
    assert_eq!(thread["id"], 1);
    let backup = dir.path().join("forum-backup.sqlite");
    forum::backup(&s, &backup).unwrap();
    assert!(forum::backup(&s, &backup).is_err());
    s.update_named_control("forum", &json!({"enabled":false}), at)
        .unwrap();
    err(
        run(&s, Method::GET, "threads", &[], Value::Null, "", at),
        503,
    );
    s.update_named_control("forum", &json!({"enabled":true}), at)
        .unwrap();
    assert_eq!(
        run(&s, Method::GET, "threads/1", &[], Value::Null, "", at).unwrap()["thread"]["body"],
        "薄荷需要适宜光照"
    );
    assert_eq!(
        rusqlite::Connection::open(backup)
            .unwrap()
            .query_row("SELECT count(*) FROM posts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn authenticated_threads_replies_search_and_cursor_pagination() {
    let (_dir, s, a, b) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    err(
        run(
            &s,
            Method::POST,
            "threads",
            &[],
            body("garden-a", "hello"),
            &b,
            0,
        ),
        401,
    );
    err(
        run(
            &s,
            Method::POST,
            "threads",
            &[],
            json!({"id":"garden-a","title":"x","body":"x"}),
            &a,
            0,
        ),
        400,
    );
    let p = run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "向日葵开花了"),
        &a,
        0,
    )
    .unwrap();
    assert_eq!(p["id"], 1);
    err(
        run(
            &s,
            Method::POST,
            "threads",
            &[],
            body("garden-a", "spam"),
            &a,
            1,
        ),
        429,
    );
    run(
        &s,
        Method::POST,
        "threads/1/replies",
        &[],
        body("garden-b", "瓢虫吃蚜虫，蜘蛛也会捕食"),
        &b,
        1,
    )
    .unwrap();
    err(
        run(
            &s,
            Method::POST,
            "threads/2/replies",
            &[],
            body("garden-a", "bad target"),
            &a,
            10000,
        ),
        404,
    );
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "水箱小记"),
        &a,
        60000,
    )
    .unwrap();
    let page = run(
        &s,
        Method::GET,
        "threads",
        &[("limit", "1")],
        Value::Null,
        "",
        60000,
    )
    .unwrap();
    assert_eq!(page["threads"][0]["id"], 3);
    assert_eq!(page["nextCursor"], 3);
    let page = run(
        &s,
        Method::GET,
        "threads",
        &[("limit", "1"), ("before", "3")],
        Value::Null,
        "",
        60000,
    )
    .unwrap();
    assert_eq!(page["threads"][0]["id"], 1);
    assert!(page["nextCursor"].is_null());
    for q in ["向日葵", "薄荷", "蜘蛛", "养育", "\" OR 1=1 --"] {
        let page = run(
            &s,
            Method::GET,
            "threads",
            &[("q", q)],
            Value::Null,
            "",
            60000,
        )
        .unwrap();
        let n = page["threads"].as_array().unwrap().len();
        assert_eq!(
            n,
            if q == "向日葵" || q == "蜘蛛" {
                1
            } else if q == "薄荷" || q == "养育" {
                2
            } else {
                0
            },
            "query {q}"
        );
    }
    let thread = run(&s, Method::GET, "threads/1", &[], Value::Null, "", 60000).unwrap();
    assert_eq!(thread["thread"]["replies"], 1);
    assert_eq!(thread["replies"][0]["author"], "garden-b");
    assert_eq!(thread["externalContent"], true);
}
#[test]
fn likes_and_follows_are_idempotent_and_follow_feed_is_filtered() {
    let (_dir, s, a, b) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "A"),
        &a,
        0,
    )
    .unwrap();
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-b", "B"),
        &b,
        0,
    )
    .unwrap();
    let like = json!({"id":"garden-b","clientVersion":"0.8.7","postId":1,"liked":true});
    for _ in 0..3 {
        assert_eq!(
            run(&s, Method::PUT, "likes", &[], like.clone(), &b, 1).unwrap()["likes"],
            1
        );
    }
    let mut unlike = like;
    unlike["liked"] = json!(false);
    assert_eq!(
        run(&s, Method::PUT, "likes", &[], unlike, &b, 2000).unwrap()["likes"],
        0
    );
    let follow =
        json!({"id":"garden-b","clientVersion":"0.8.7","targetId":"garden-a","following":true});
    for _ in 0..3 {
        run(&s, Method::PUT, "follows", &[], follow.clone(), &b, 1).unwrap();
    }
    let profile = run(
        &s,
        Method::GET,
        "profiles/garden-a",
        &[],
        Value::Null,
        "",
        1,
    )
    .unwrap();
    assert_eq!(profile["followers"], 1);
    let feed = run(
        &s,
        Method::GET,
        "threads",
        &[("followingOf", "garden-b")],
        Value::Null,
        "",
        1,
    )
    .unwrap();
    assert_eq!(feed["threads"].as_array().unwrap().len(), 1);
    assert_eq!(feed["threads"][0]["author"], "garden-a");
    err(
        run(
            &s,
            Method::PUT,
            "follows",
            &[],
            json!({"id":"garden-b","clientVersion":"0.8.7","targetId":"garden-b","following":true}),
            &b,
            2000,
        ),
        400,
    );
}
#[tokio::test]
async fn public_routes_enforce_closed_gate_authentication_and_body_limit() {
    let (_dir, s, a, _) = fixture();
    let app = router(App {
        store: s.clone(),
        ingestion: Some(Ingestion::new(s.clone())),
        release_id: None,
        release_root: None,
    });
    async fn req(app: axum::Router, path: &str, method: &str, body: String) -> (StatusCode, Value) {
        let r = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method(method)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = r.status();
        let b = r.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&b).unwrap())
    }
    let (code, v) = req(app.clone(), "/api/forum/threads", "GET", String::new()).await;
    assert_eq!(code, 503);
    assert_eq!(v["error"], "未开放论坛");
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    assert_eq!(
        req(
            app.clone(),
            "/api/forum/threads",
            "POST",
            body("garden-a", "x").to_string()
        )
        .await
        .0,
        401
    );
    assert_eq!(
        req(app.clone(), "/api/forum/threads", "POST", "x".repeat(17000))
            .await
            .0,
        413
    );
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/forum/threads")
                .method("POST")
                .header("authorization", format!("Bearer {a}"))
                .body(Body::from(
                    body("garden-a", "<script>alert(1)</script>").to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let (_, v) = req(app, "/api/presentation", "GET", String::new()).await;
    assert_eq!(v["logoVersion"], "0.8.6");
    assert!(v["logo"].as_str().unwrap().contains("闲看生机"));
}
