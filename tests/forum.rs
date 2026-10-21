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

#[test]
fn names_follow_current_box_names_without_changing_post_authors_or_follow_ids() {
    let (_dir, s, a, b) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    let cases: Vec<Value> = serde_json::from_str(include_str!("fixtures/scores.json")).unwrap();
    let mut entry = json!({"id":"garden-a","kind":"land","state":cases[0]["state"],"reportedAt":"2026-10-20T00:00:00.000Z","reportRevision":"names-1"});
    entry["state"]["name"] = json!("薄荷小院 <花园>");
    assert!(s.commit_entry(&entry, None).unwrap());
    let post = run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "新叶"),
        &a,
        1000,
    )
    .unwrap();
    assert_eq!(post["author"], "garden-a");
    assert_eq!(post["authorName"], "薄荷小院 <花园>");
    let reply = run(
        &s,
        Method::POST,
        "threads/1/replies",
        &[],
        body("garden-b", "我也来看看"),
        &b,
        2000,
    )
    .unwrap();
    assert_eq!(reply["authorName"], "未命名的生态箱");
    run(
        &s,
        Method::PUT,
        "follows",
        &[],
        json!({"id":"garden-b","clientVersion":"0.8.7","targetId":"garden-a","following":true}),
        &b,
        3000,
    )
    .unwrap();
    let profile = run(
        &s,
        Method::GET,
        "profiles/garden-b",
        &[],
        Value::Null,
        "",
        3000,
    )
    .unwrap();
    assert_eq!(profile["following"], json!(["garden-a"]));
    assert_eq!(
        profile["followingProfiles"],
        json!([{"id":"garden-a","name":"薄荷小院 <花园>"}])
    );
    let previous = entry.clone();
    entry["reportRevision"] = json!("names-2");
    entry["state"]["name"] = json!("新叶小院");
    assert!(s.commit_entry(&entry, Some(&previous)).unwrap());
    let thread = run(&s, Method::GET, "threads/1", &[], Value::Null, "", 4000).unwrap();
    assert_eq!(thread["thread"]["author"], "garden-a");
    assert_eq!(thread["thread"]["authorName"], "新叶小院");
    let listed = run(
        &s,
        Method::GET,
        "threads",
        &[("author", "garden-a")],
        Value::Null,
        "",
        4000,
    )
    .unwrap();
    assert_eq!(listed["threads"][0]["authorName"], "新叶小院");
    let profile = run(
        &s,
        Method::GET,
        "profiles/garden-a",
        &[],
        Value::Null,
        "",
        4000,
    )
    .unwrap();
    assert_eq!(profile["id"], "garden-a");
    assert_eq!(profile["name"], "新叶小院");
    let previous = entry.clone();
    entry["reportRevision"] = json!("names-3");
    entry["state"].as_object_mut().unwrap().remove("name");
    assert!(s.commit_entry(&entry, Some(&previous)).unwrap());
    assert_eq!(
        run(&s, Method::GET, "threads/1", &[], Value::Null, "", 5000).unwrap()["thread"]["authorName"],
        "未命名地箱"
    );
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

#[test]
fn recommendations_preserve_pages_during_engagement_and_expire_with_refresh() {
    let (_dir, s, a, b) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    for (n, t) in [(1, 0), (2, 60000), (3, 120000)] {
        run(
            &s,
            Method::POST,
            "threads",
            &[],
            body("garden-a", &format!("leaf {n}")),
            &a,
            t,
        )
        .unwrap();
    }
    run(
        &s,
        Method::PUT,
        "likes",
        &[],
        json!({"id":"garden-b","clientVersion":"0.8.7","postId":1,"liked":true}),
        &b,
        120001,
    )
    .unwrap();
    let q = [("sort", "recommended"), ("limit", "1")];
    let first = run(&s, Method::GET, "threads", &q, Value::Null, "", 120002).unwrap();
    assert_eq!(first["threads"][0]["id"], 1);
    let cursor = first["nextCursor"].as_i64().unwrap().to_string();
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "new arrival"),
        &a,
        180000,
    )
    .unwrap();
    run(
        &s,
        Method::PUT,
        "likes",
        &[],
        json!({"id":"garden-b","clientVersion":"0.8.7","postId":2,"liked":true}),
        &b,
        180001,
    )
    .unwrap();
    let nextq = [
        ("sort", "recommended"),
        ("limit", "1"),
        ("before", cursor.as_str()),
    ];
    let next = run(&s, Method::GET, "threads", &nextq, Value::Null, "", 180002).unwrap();
    assert_eq!(next["threads"][0]["id"], 3);
    let bad = [
        ("sort", "recommended"),
        ("author", "garden-b"),
        ("before", cursor.as_str()),
    ];
    err(
        run(&s, Method::GET, "threads", &bad, Value::Null, "", 180003),
        400,
    );
    let latest = run(&s, Method::GET, "threads", &[], Value::Null, "", 180003).unwrap();
    assert_eq!(latest["threads"][0]["id"], 4);
    let refreshed = run(&s, Method::GET, "threads", &q, Value::Null, "", 180003).unwrap();
    assert_eq!(refreshed["threads"][0]["id"], 2);
    err(
        run(&s, Method::GET, "threads", &nextq, Value::Null, "", 720002),
        410,
    );
}
#[test]
fn thread_opens_count_views_but_lists_and_reply_pages_do_not_and_backup_preserves_them() {
    let (dir, s, a, _) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "leaf"),
        &a,
        0,
    )
    .unwrap();
    assert_eq!(
        run(&s, Method::GET, "threads", &[], Value::Null, "", 0).unwrap()["threads"][0]["views"],
        0
    );
    assert_eq!(
        run(&s, Method::GET, "threads/1", &[], Value::Null, "", 1).unwrap()["thread"]["views"],
        1
    );
    assert_eq!(
        run(
            &s,
            Method::GET,
            "threads/1",
            &[("before", "99")],
            Value::Null,
            "",
            2
        )
        .unwrap()["thread"]["views"],
        1
    );
    assert_eq!(
        run(&s, Method::GET, "threads/1", &[], Value::Null, "", 3).unwrap()["thread"]["views"],
        2
    );
    let path = dir.path().join("backup.sqlite");
    forum::backup(&s, &path).unwrap();
    let c = rusqlite::Connection::open(path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT at FROM activity WHERE author='@thread:1' AND action='views'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn oldest_replies_paginate_by_creation_time_with_stable_ties_and_keep_legacy_order() {
    let (_dir, s, a, b) = fixture();
    s.update_named_control("forum", &json!({"enabled":true}), 0)
        .unwrap();
    run(
        &s,
        Method::POST,
        "threads",
        &[],
        body("garden-a", "Opening"),
        &a,
        0,
    )
    .unwrap();
    for (id, token, time, text) in [
        ("garden-b", &b, 30000, "Later"),
        ("garden-a", &a, 10000, "First"),
        ("garden-a", &a, 20000, "Second"),
        ("garden-a", &a, 30000, "Same time"),
    ] {
        run(
            &s,
            Method::POST,
            "threads/1/replies",
            &[],
            body(id, text),
            token,
            time,
        )
        .unwrap();
    }
    let page = run(
        &s,
        Method::GET,
        "threads/1",
        &[("order", "oldest"), ("limit", "2")],
        Value::Null,
        "",
        40000,
    )
    .unwrap();
    assert_eq!(
        page["replies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["body"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["First", "Second"]
    );
    assert_eq!(page["nextCursor"], 4);
    assert_eq!(page["thread"]["views"], 1);
    run(
        &s,
        Method::POST,
        "threads/1/replies",
        &[],
        body("garden-a", "Newest"),
        &a,
        40000,
    )
    .unwrap();
    let page = run(
        &s,
        Method::GET,
        "threads/1",
        &[("order", "oldest"), ("limit", "2"), ("after", "4")],
        Value::Null,
        "",
        40000,
    )
    .unwrap();
    assert_eq!(
        page["replies"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["body"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Later", "Same time"]
    );
    assert_eq!(page["nextCursor"], 5);
    assert_eq!(page["thread"]["views"], 1);
    let page = run(
        &s,
        Method::GET,
        "threads/1",
        &[("order", "oldest"), ("after", "5")],
        Value::Null,
        "",
        40000,
    )
    .unwrap();
    assert_eq!(page["replies"][0]["body"], "Newest");
    assert!(page["nextCursor"].is_null());
    let legacy = run(
        &s,
        Method::GET,
        "threads/1",
        &[("limit", "2")],
        Value::Null,
        "",
        40000,
    )
    .unwrap();
    assert_eq!(legacy["replies"][0]["body"], "Newest");
    assert_eq!(legacy["replies"][1]["body"], "Same time");
    for query in [
        vec![("order", "oldest"), ("after", "1")],
        vec![("order", "oldest"), ("before", "5")],
        vec![("order", "invalid")],
    ] {
        err(
            run(&s, Method::GET, "threads/1", &query, Value::Null, "", 40000),
            400,
        );
    }
}
