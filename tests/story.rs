use axum::http::Method;
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Barrier},
    thread,
};
use tempfile::TempDir;
use terrarium_server::{
    forum,
    ingestion::now_ms,
    policy::iso,
    store::{LegacySnapshot, Store},
};

fn entry(id: &str, kind: &str) -> Value {
    let mut state: Value =
        serde_json::from_str::<Value>(include_str!("fixtures/reports.json")).unwrap()[kind].clone();
    state["updatedAtMs"] = json!(now_ms());
    if kind == "land" {
        let p = &mut state["plants"][0];
        p["species"] = json!("sunflower");
        p["x"] = json!(4);
        p["y"] = json!(1);
        p["growthHours"] = json!(18);
        p["stage"] = json!(3);
    }
    json!({"id":id,"kind":kind,"state":state,"score":0,"reportedAt":iso(now_ms()),"reportRevision":"first"})
}
fn fixture() -> (TempDir, Store, String, i64) {
    let dir = TempDir::new().unwrap();
    let store = Store::open(dir.path().join("data.sqlite")).unwrap();
    store
        .update_named_control("forum", &json!({"enabled":true}), now_ms())
        .unwrap();
    let token = store.register(Some("story-author")).unwrap().unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    store.register(Some("garden-first")).unwrap();
    store.register(Some("garden-second")).unwrap();
    let post = forum::execute(&store,&Method::POST,"threads",&HashMap::new(),&json!({"id":"story-author","title":"A garden note","body":"An optional story","clientVersion":"0.8.7"}),&token,now_ms()).unwrap();
    (dir, store, token, post["id"].as_i64().unwrap())
}
fn participate(s: &Store, root: i64, id: &str) {
    let conn = forum::open(s).unwrap();
    conn.execute("INSERT INTO posts(thread,author,title,body,created) VALUES(?1,?2,'','I will take part',?3)",params![root,id,now_ms()-1000]).unwrap();
}
fn notices(s: &Store) -> Vec<Value> {
    s.list_announcements().unwrap()["announcements"]
        .as_array()
        .unwrap()
        .clone()
}
fn configure(s: &Store, root: i64) {
    s.update_story(&json!({"enabled":true,"threadId":root}))
        .unwrap();
}

#[test]
fn opt_in_position_species_and_growth_threshold_produce_one_private_notice_across_restart_and_export()
 {
    let (dir, s, _, root) = fixture();
    configure(&s, root);
    let mut first = entry("garden-first", "land");
    assert!(s.commit_entry(&first, None).unwrap());
    assert!(notices(&s).is_empty());
    participate(&s, root, "garden-first");
    for (key, value) in [
        ("x", json!(3)),
        ("species", json!("mint")),
        ("growthHours", json!(17.99)),
    ] {
        let mut next = first.clone();
        next["state"]["plants"][0][key] = value;
        next["reportRevision"] = json!(format!("wrong-{key}"));
        assert!(s.commit_entry(&next, Some(&first)).unwrap());
        first = next;
        assert!(notices(&s).is_empty());
        let mut reset = entry("garden-first", "land");
        reset["state"]["plants"][0]["growthHours"] = json!(0);
        reset["reportRevision"] = json!(format!("reset-{key}"));
        assert!(s.commit_entry(&reset, Some(&first)).unwrap());
        first = reset;
    }
    let mut completed = entry("garden-first", "land");
    completed["reportRevision"] = json!("complete");
    assert!(s.commit_entry(&completed, Some(&first)).unwrap());
    let sent = notices(&s);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["targets"], json!(["garden-first"]));
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 1);
    assert!(!s.commit_entry(&completed, Some(&first)).unwrap());
    let restored = Store::existing(dir.path().join("data.sqlite")).unwrap();
    restored
        .revoke_announcement(sent[0]["id"].as_u64().unwrap())
        .unwrap();
    let mut again = completed.clone();
    again["reportRevision"] = json!("after-restart");
    assert!(restored.commit_entry(&again, Some(&completed)).unwrap());
    assert!(notices(&restored).is_empty());
    let output = dir.path().join("export");
    restored.export(&output).unwrap();
    assert!(output.join("story.json").is_file());
    let imported = Store::open(dir.path().join("imported.sqlite")).unwrap();
    imported
        .import(LegacySnapshot::read(&output).unwrap())
        .unwrap();
    assert_eq!(
        imported.story_status().unwrap(),
        restored.story_status().unwrap()
    );
    let backup = dir.path().join("backup.sqlite");
    restored.backup(&backup).unwrap();
    assert_eq!(
        Store::existing(backup).unwrap().story_status().unwrap(),
        restored.story_status().unwrap()
    );
}
#[test]
fn disabled_water_and_stale_reports_do_not_receive_story_notices() {
    let (_dir, s, _, root) = fixture();
    participate(&s, root, "garden-first");
    let first = entry("garden-first", "land");
    assert!(s.commit_entry(&first, None).unwrap());
    assert!(notices(&s).is_empty());
    configure(&s, root);
    let mut second = first.clone();
    second["reportRevision"] = json!("fresh");
    second["state"]["updatedAtMs"] = json!(0);
    assert!(s.commit_entry(&second, Some(&first)).unwrap());
    assert!(notices(&s).is_empty());
    let water = entry("garden-second", "water");
    participate(&s, root, "garden-second");
    assert!(s.commit_entry(&water, None).unwrap());
    assert!(notices(&s).is_empty());
    s.update_story(&json!({"enabled":false})).unwrap();
    let mut later = entry("garden-first", "land");
    later["reportRevision"] = json!("off");
    assert!(s.commit_entry(&later, Some(&second)).unwrap());
    assert!(notices(&s).is_empty());
}
#[test]
fn failed_story_write_rolls_back_both_report_and_notice_and_can_retry() {
    let (dir, s, _, root) = fixture();
    configure(&s, root);
    participate(&s, root, "garden-first");
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_story BEFORE INSERT ON legacy_documents WHEN new.name='story.json' BEGIN SELECT RAISE(ABORT,'story write failed'); END;").unwrap();
    let e = entry("garden-first", "land");
    assert!(s.commit_entry(&e, None).is_err());
    assert!(s.get("garden-first").unwrap().is_none());
    assert!(notices(&s).is_empty());
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 0);
    conn.execute_batch("DROP TRIGGER fail_story;").unwrap();
    assert!(s.commit_entry(&e, None).unwrap());
    assert_eq!(notices(&s).len(), 1);
}
#[test]
fn concurrent_completions_share_the_global_cursor_and_do_not_repeat_per_box() {
    let (_dir, s, _, root) = fixture();
    configure(&s, root);
    for id in ["garden-first", "garden-second"] {
        participate(&s, root, id);
    }
    let barrier = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|i| {
            let s = s.clone();
            let b = barrier.clone();
            thread::spawn(move || {
                let e = entry(
                    if i % 2 == 0 {
                        "garden-first"
                    } else {
                        "garden-second"
                    },
                    "land",
                );
                b.wait();
                s.commit_entry(&e, None).unwrap()
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|w| w.join().unwrap())
            .filter(|accepted| *accepted)
            .count(),
        2
    );
    let sent = notices(&s);
    assert_eq!(sent.len(), 2);
    assert!(sent[0]["id"].as_u64().unwrap() < sent[1]["id"].as_u64().unwrap());
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 2);
}
#[test]
fn configuration_requires_a_real_root_and_cannot_retarget_after_delivery() {
    let (_dir, s, _, root) = fixture();
    for input in [
        json!({"enabled":true}),
        json!({"threadId":999}),
        json!({"threadId":-1}),
        json!({"enabled":"yes"}),
        json!({"x":4}),
    ] {
        assert!(s.update_story(&input).is_err());
    }
    configure(&s, root);
    participate(&s, root, "garden-first");
    assert!(
        s.commit_entry(&entry("garden-first", "land"), None)
            .unwrap()
    );
    assert!(
        s.update_story(&json!({"enabled":false,"threadId":null}))
            .is_err()
    );
    assert_eq!(
        s.update_story(&json!({"enabled":false})).unwrap()["enabled"],
        false
    );
}

fn submit(s: &Store, e: &Value) -> anyhow::Result<bool> {
    let previous = s.get(e["id"].as_str().unwrap())?;
    s.commit_entry(e, previous.as_ref())
}
fn granary_entry(id: &str) -> Value {
    let mut e = entry(id, "land");
    e["state"]["nests"] = json!([{"id":"nest-1","x":2,"y":2,"depth":5,"food":3}]);
    e["state"]["bugs"] = json!([{"id":"ant-1","species":"ant","x":2,"y":2,"ageHours":1,"energy":60,"hunger":0,"role":"queen","inside":{"nestId":"nest-1","x":1,"y":1}}]);
    e
}
fn next_thread(s: &Store, token: &str) -> i64 {
    let p = forum::execute(s,&Method::POST,"threads",&HashMap::new(),&json!({"id":"story-author","title":"The granary","body":"An optional continuation","clientVersion":"0.8.7"}),token,now_ms()+60001).unwrap();
    let root = p["id"].as_i64().unwrap();
    s.update_granary_story(&json!({"enabled":true,"threadId":root}))
        .unwrap();
    root
}
#[test]
fn granary_requires_prior_window_delivery_and_a_separate_opt_in_report() {
    let (_dir, s, token, window) = fixture();
    configure(&s, window);
    let granary = next_thread(&s, &token);
    participate(&s, granary, "garden-first");
    let mut e = granary_entry("garden-first");
    assert!(submit(&s, &e).unwrap());
    assert!(notices(&s).is_empty());
    participate(&s, window, "garden-first");
    e["reportRevision"] = json!("window");
    assert!(submit(&s, &e).unwrap());
    assert_eq!(notices(&s).len(), 1);
    assert_eq!(s.granary_story_status().unwrap()["completedBoxes"], 0);
    // The plant may be collected after the first milestone; the saved delivery is the prerequisite.
    e["state"]["plants"] = json!([]);
    e["reportRevision"] = json!("granary");
    assert!(submit(&s, &e).unwrap());
    let sent = notices(&s);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1]["targets"], json!(["garden-first"]));
    assert_eq!(
        sent[1]["id"].as_u64(),
        sent[0]["id"].as_u64().map(|n| n + 1)
    );
    assert_eq!(s.granary_story_status().unwrap()["completedBoxes"], 0);
    assert_eq!(
        s.granary_story_status().unwrap()["progressCounts"],
        json!([1, 0, 0, 0])
    );
}
#[test]
fn granary_checks_food_and_ant_in_the_same_nest_and_retries_after_write_failure() {
    let (dir, s, token, window) = fixture();
    configure(&s, window);
    participate(&s, window, "garden-first");
    s.commit_entry(&entry("garden-first", "land"), None)
        .unwrap();
    let granary = next_thread(&s, &token);
    let e = granary_entry("garden-first");
    submit(&s, &e).unwrap();
    assert_eq!(notices(&s).len(), 1);
    participate(&s, granary, "garden-first");
    for (i, mut bad) in (0..7).map(|i| (i, e.clone())) {
        match i {
            0 => bad["state"]["nests"][0]["food"] = json!(2),
            1 => bad["state"]["bugs"][0]["inside"]["nestId"] = json!("other-nest"),
            2 => bad["state"]["bugs"][0]
                .as_object_mut()
                .unwrap()
                .remove("inside")
                .map(|_| ())
                .unwrap(),
            3 => bad["state"]["bugs"][0]["energy"] = json!(0),
            4 => bad["state"]["bugs"][0]["hunger"] = json!(100),
            5 => bad["state"]["updatedAtMs"] = json!(0),
            _ => bad = entry("garden-first", "water"),
        }
        bad["reportRevision"] = json!(format!("ineligible-{i}"));
        submit(&s, &bad).unwrap();
        assert_eq!(notices(&s).len(), 1);
    }
    s.update_granary_story(&json!({"enabled":false})).unwrap();
    let mut ready = e.clone();
    ready["reportRevision"] = json!("disabled");
    submit(&s, &ready).unwrap();
    assert_eq!(notices(&s).len(), 1);
    s.update_granary_story(&json!({"enabled":true})).unwrap();
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_granary BEFORE INSERT ON legacy_documents WHEN new.name='story-granary.json' BEGIN SELECT RAISE(ABORT,'granary write failed'); END;").unwrap();
    ready["reportRevision"] = json!("retry");
    assert!(submit(&s, &ready).is_err());
    assert_eq!(
        s.get("garden-first").unwrap().unwrap()["reportRevision"],
        json!("disabled")
    );
    assert_eq!(notices(&s).len(), 1);
    conn.execute_batch("DROP TRIGGER fail_granary;").unwrap();
    assert!(submit(&s, &ready).unwrap());
    assert_eq!(notices(&s).len(), 2);
}
#[test]
fn granary_completion_survives_restart_export_and_notice_revocation() {
    let (dir, s, token, window) = fixture();
    configure(&s, window);
    let granary = next_thread(&s, &token);
    for root in [window, granary] {
        participate(&s, root, "garden-first");
    }
    let mut e = granary_entry("garden-first");
    submit(&s, &e).unwrap();
    e["reportRevision"] = json!("next");
    submit(&s, &e).unwrap();
    let output = dir.path().join("export");
    s.export(&output).unwrap();
    assert!(output.join("story-granary.json").is_file());
    let restored = Store::open(dir.path().join("imported.sqlite")).unwrap();
    restored
        .import(LegacySnapshot::read(&output).unwrap())
        .unwrap();
    assert_eq!(
        restored.granary_story_status().unwrap(),
        s.granary_story_status().unwrap()
    );
    let reopened = Store::existing(dir.path().join("data.sqlite")).unwrap();
    assert_eq!(
        reopened.granary_story_status().unwrap(),
        s.granary_story_status().unwrap()
    );
    assert!(
        reopened
            .update_granary_story(&json!({"threadId":window}))
            .is_err()
    );
    for n in notices(&reopened) {
        reopened
            .revoke_announcement(n["id"].as_u64().unwrap())
            .unwrap();
    }
    e["reportRevision"] = json!("again");
    submit(&reopened, &e).unwrap();
    assert!(notices(&reopened).is_empty());
}

fn nursery(e: &mut Value) {
    e["state"]["eggs"] = json!([{"id":"egg-1","species":"ant","x":2,"y":2,"laidAtMs":now_ms()-1000,"hatchesAtMs":now_ms()+3600000,"inside":{"nestId":"nest-1","x":2,"y":2}}]);
}
fn advance(s: &Store, e: &mut Value, revision: &str) {
    e["reportRevision"] = json!(revision);
    e["state"]["updatedAtMs"] = json!(now_ms());
    assert!(submit(s, e).unwrap());
}
fn reply(s: &Store, root: i64, author: &str, body: &str) {
    forum::open(s)
        .unwrap()
        .execute(
            "INSERT INTO posts(thread,author,title,body,created) VALUES(?1,?2,'',?3,?4)",
            params![root, author, body, now_ms()],
        )
        .unwrap();
}
#[test]
fn investigation_pages_unlock_in_order_and_only_a_new_answer_from_the_same_box_finishes() {
    let (_dir, s, token, window) = fixture();
    configure(&s, window);
    let root = next_thread(&s, &token);
    participate(&s, window, "garden-first");
    participate(&s, root, "garden-first");
    let mut e = granary_entry("garden-first");
    nursery(&mut e);
    e["state"]["nests"][0]["size"] = json!(5);
    e["state"]["nests"][0]["entrances"] = json!([{"x":3,"y":2}]);
    reply(&s, root, "garden-first", "[续记]北窗仍在观察");
    advance(&s, &mut e, "window");
    advance(&s, &mut e, "granary");
    assert_eq!(notices(&s).len(), 2);
    advance(&s, &mut e, "nursery");
    assert_eq!(notices(&s).len(), 3);
    advance(&s, &mut e, "door");
    assert_eq!(notices(&s).len(), 4);
    advance(&s, &mut e, "old-answer-does-not-count");
    assert_eq!(notices(&s).len(), 4);
    reply(&s, root, "garden-second", "[续记]北窗仍在观察");
    reply(&s, root, "garden-first", "[续记]猜错了");
    advance(&s, &mut e, "other-and-wrong-answer");
    assert_eq!(notices(&s).len(), 4);
    reply(&s, root, "garden-first", "我读到的是：[续记]北窗仍在观察");
    let conn = s.reader().unwrap();
    let cursor: Value = serde_json::from_str(&conn.query_row("SELECT CAST(content AS TEXT) FROM legacy_documents WHERE name='story-granary.json'",[],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    let after = cursor["deliveries"]["garden-first"]["completedAt"]
        .as_str()
        .unwrap();
    e["state"]["updatedAtMs"] = json!(terrarium_server::policy::at(&json!(after)).unwrap() - 1);
    e["reportRevision"] = json!("stale-answer-report");
    submit(&s, &e).unwrap();
    assert_eq!(notices(&s).len(), 4);
    advance(&s, &mut e, "answer");
    let sent = notices(&s);
    assert_eq!(sent.len(), 5);
    assert_eq!(
        s.granary_story_status().unwrap()["progressCounts"],
        json!([1, 1, 1, 1])
    );
    assert_eq!(s.granary_story_status().unwrap()["completedBoxes"], 1);
    let code: String = sent[1..4]
        .iter()
        .flat_map(|n| {
            n["text"]
                .as_str()
                .unwrap()
                .lines()
                .take(2)
                .map(|line| line.chars().next().unwrap())
        })
        .collect();
    assert_eq!(code, "北窗仍在观察");
    for n in &sent {
        assert!(n["text"].as_str().unwrap().encode_utf16().count() <= 2000);
        assert_eq!(n["targets"], json!(["garden-first"]));
    }
    advance(&s, &mut e, "finished");
    assert_eq!(notices(&s).len(), 5);
}
#[test]
fn nursery_and_second_door_require_residents_in_the_matching_colony() {
    let (_dir, s, token, window) = fixture();
    configure(&s, window);
    let root = next_thread(&s, &token);
    participate(&s, window, "garden-first");
    participate(&s, root, "garden-first");
    let mut e = granary_entry("garden-first");
    advance(&s, &mut e, "window");
    advance(&s, &mut e, "granary");
    nursery(&mut e);
    e["state"]["eggs"][0]["inside"]["nestId"] = json!("other-nest");
    advance(&s, &mut e, "wrong-nursery");
    assert_eq!(notices(&s).len(), 2);
    e["state"]["eggs"] = json!([]);
    let mut child = e["state"]["bugs"][0].clone();
    child["id"] = json!("ant-2");
    child["role"] = json!("worker");
    child["ageHours"] = json!(6);
    child["inside"]["x"] = json!(2);
    e["state"]["bugs"].as_array_mut().unwrap().push(child);
    advance(&s, &mut e, "adult-worker");
    assert_eq!(notices(&s).len(), 2);
    e["state"]["bugs"][1]["ageHours"] = json!(5.99);
    advance(&s, &mut e, "young-worker");
    assert_eq!(notices(&s).len(), 3);
    e["state"]["nests"][0]["size"] = json!(5);
    e["state"]["nests"][0]["entrances"] = json!([]);
    advance(&s, &mut e, "one-entrance");
    assert_eq!(notices(&s).len(), 3);
    e["state"]["nests"][0]["entrances"] = json!([{"x":3,"y":2}]);
    for b in e["state"]["bugs"].as_array_mut().unwrap() {
        b.as_object_mut().unwrap().remove("inside");
    }
    advance(&s, &mut e, "empty-nest");
    assert_eq!(notices(&s).len(), 3);
    e["state"]["bugs"][0]["inside"] = json!({"nestId":"nest-1","x":1,"y":1});
    advance(&s, &mut e, "second-door");
    assert_eq!(notices(&s).len(), 4);
}

fn seedbed_thread(s: &Store, token: &str) -> i64 {
    let post = forum::execute(s, &Method::POST, "threads", &HashMap::new(),
        &json!({"id":"story-author","title":"The potting bench","body":"An optional beginner mystery","clientVersion":"0.8.7"}), token, now_ms()+60001).unwrap();
    let root = post["id"].as_i64().unwrap();
    s.update_seedbed_story(&json!({"enabled":true,"threadId":root}))
        .unwrap();
    root
}
fn seedbed_entry() -> Value {
    let mut e = granary_entry("garden-first");
    e["state"]["plants"] = json!([{
        "id":"clover", "species":"clover", "x":4, "y":0, "moisture":50,
        "growthHours":18, "stage":3, "health":100, "dry":false
    }]);
    e["state"]["nests"][0]["food"] = json!(5);
    e
}
#[test]
fn seedbed_is_independent_opt_in_and_teaches_in_order_from_existing_ecosystems() {
    let (_dir, s, token, _) = fixture();
    let root = seedbed_thread(&s, &token);
    let mut e = seedbed_entry();
    advance(&s, &mut e, "not-joined");
    assert!(notices(&s).is_empty());
    participate(&s, root, "garden-first");
    advance(&s, &mut e, "letter-one");
    assert_eq!(notices(&s).len(), 1);
    assert_eq!(s.story_status().unwrap()["completedBoxes"], 0);
    assert_eq!(s.granary_story_status().unwrap()["completedBoxes"], 0);
    for (i, (field, value)) in [
        ("health", json!(59.99)),
        ("moisture", json!(19.99)),
        ("moisture", json!(80.01)),
        ("species", json!("fern")),
    ]
    .into_iter()
    .enumerate()
    {
        let mut bad = e.clone();
        bad["state"]["plants"][0][field] = value;
        advance(&s, &mut bad, &format!("bad-soil-{i}"));
        assert_eq!(notices(&s).len(), 1);
    }
    e["state"]["plants"][0]["growthHours"] = json!(0);
    advance(&s, &mut e, "suitable-no-watering-needed");
    assert_eq!(notices(&s).len(), 2);
    e["state"]["plants"][0]["growthHours"] = json!(5.99);
    advance(&s, &mut e, "wait-for-growth");
    assert_eq!(notices(&s).len(), 2);
    e["state"]["plants"][0]["growthHours"] = json!(6);
    advance(&s, &mut e, "grown");
    assert_eq!(notices(&s).len(), 3);
    e["state"]["plants"][0]["growthHours"] = json!(17.99);
    advance(&s, &mut e, "wait-for-flower");
    assert_eq!(notices(&s).len(), 3);
    e["state"]["plants"][0]["growthHours"] = json!(18);
    advance(&s, &mut e, "flower");
    assert_eq!(notices(&s).len(), 4);
    e["state"]["bugs"][0]["inside"]["nestId"] = json!("other");
    advance(&s, &mut e, "food-in-empty-room");
    assert_eq!(notices(&s).len(), 4);
    e["state"]["bugs"][0]["inside"]["nestId"] = json!("nest-1");
    e["state"]["nests"][0]["food"] = json!(2);
    advance(&s, &mut e, "not-enough-food");
    assert_eq!(notices(&s).len(), 4);
    e["state"]["nests"][0]["food"] = json!(3);
    advance(&s, &mut e, "resident-and-food");
    assert_eq!(
        s.seedbed_story_status().unwrap()["progressCounts"],
        json!([1, 1, 1, 1, 1, 0])
    );
    let sent = notices(&s);
    assert_eq!(sent.len(), 5);
    let answer: String = sent[..3]
        .iter()
        .flat_map(|n| {
            n["text"]
                .as_str()
                .unwrap()
                .lines()
                .take(2)
                .map(|line| line.chars().next().unwrap())
        })
        .collect();
    assert_eq!(answer, "有人接过水壶");
    assert!(sent[0]["text"].as_str().unwrap().contains("targetMoisture"));
    assert!(sent[1]["text"].as_str().unwrap().contains("夜间"));
    assert!(sent[3]["text"].as_str().unwrap().contains("hunger"));
}
#[test]
fn seedbed_answers_require_same_box_thread_and_new_reply_and_survive_backup_and_export() {
    let (dir, s, token, other_thread) = fixture();
    let root = seedbed_thread(&s, &token);
    participate(&s, root, "garden-first");
    let mut e = seedbed_entry();
    reply(&s, root, "garden-first", "[接班]有人接过水壶");
    for step in 1..=5 {
        advance(&s, &mut e, &format!("page-{step}"));
        assert_eq!(notices(&s).len(), step);
    }
    advance(&s, &mut e, "old-guess");
    assert_eq!(notices(&s).len(), 5);
    reply(&s, root, "garden-second", "[接班]有人接过水壶");
    reply(&s, other_thread, "garden-first", "[接班]有人接过水壶");
    reply(&s, root, "garden-first", "[接班]水壶被人拿走");
    advance(&s, &mut e, "other-box-thread-answer");
    assert_eq!(notices(&s).len(), 5);
    reply(&s, root, "garden-first", "调查的答案是：[接班]有人接过水壶");
    e["state"]["updatedAtMs"] = json!(0);
    e["reportRevision"] = json!("stale-answer");
    submit(&s, &e).unwrap();
    assert_eq!(notices(&s).len(), 5);
    advance(&s, &mut e, "answer");
    assert_eq!(s.seedbed_story_status().unwrap()["completedBoxes"], 1);
    let sent = notices(&s);
    assert_eq!(sent.len(), 6);
    for n in &sent {
        assert_eq!(n["targets"], json!(["garden-first"]));
        assert!(n["text"].as_str().unwrap().encode_utf16().count() <= 2000);
        assert!(
            n["text"]
                .as_str()
                .unwrap()
                .contains("无需为剧情主动联系主人")
        );
        s.revoke_announcement(n["id"].as_u64().unwrap()).unwrap();
    }
    let output = dir.path().join("seedbed-export");
    s.export(&output).unwrap();
    assert!(output.join("story-seedbed.json").is_file());
    let imported = Store::open(dir.path().join("imported-seedbed.sqlite")).unwrap();
    imported
        .import(LegacySnapshot::read(&output).unwrap())
        .unwrap();
    assert_eq!(
        imported.seedbed_story_status().unwrap(),
        s.seedbed_story_status().unwrap()
    );
    let backup = dir.path().join("seedbed-backup.sqlite");
    s.backup(&backup).unwrap();
    let restored = Store::existing(&backup).unwrap();
    assert_eq!(
        restored.seedbed_story_status().unwrap()["completedBoxes"],
        1
    );
    let reopened = Store::existing(dir.path().join("data.sqlite")).unwrap();
    advance(&reopened, &mut e, "after-restart-and-revocation");
    assert!(notices(&reopened).is_empty());
    assert!(
        s.update_seedbed_story(&json!({"threadId":other_thread}))
            .is_err()
    );
}
#[test]
fn seedbed_is_disabled_by_default_and_ignores_water_and_pre_participation_snapshots() {
    let (_dir, s, _, root) = fixture();
    let initial = s.seedbed_story_status().unwrap();
    assert_eq!(initial["enabled"], false);
    assert!(
        initial["openingPost"]["text"]
            .as_str()
            .unwrap()
            .contains("旧温室")
    );
    assert!(s.update_seedbed_story(&json!({"enabled":true})).is_err());
    s.update_seedbed_story(&json!({"threadId":root})).unwrap();
    participate(&s, root, "garden-first");
    let mut e = seedbed_entry();
    advance(&s, &mut e, "disabled");
    assert!(notices(&s).is_empty());
    s.update_seedbed_story(&json!({"enabled":true})).unwrap();
    e["state"]["updatedAtMs"] = json!(0);
    e["reportRevision"] = json!("stale-join");
    submit(&s, &e).unwrap();
    assert!(notices(&s).is_empty());
    participate(&s, root, "garden-second");
    let water = entry("garden-second", "water");
    submit(&s, &water).unwrap();
    assert!(notices(&s).is_empty());
    advance(&s, &mut e, "fresh");
    assert_eq!(notices(&s).len(), 1);
    s.update_seedbed_story(&json!({"enabled":false})).unwrap();
    advance(&s, &mut e, "disabled-again");
    assert_eq!(notices(&s).len(), 1);
}
#[test]
fn seedbed_story_failure_rolls_back_report_and_notice_and_retries_once() {
    let (dir, s, token, _) = fixture();
    let root = seedbed_thread(&s, &token);
    participate(&s, root, "garden-first");
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_seedbed BEFORE INSERT ON legacy_documents WHEN new.name='story-seedbed.json' BEGIN SELECT RAISE(ABORT,'seedbed write failed'); END;").unwrap();
    let mut e = seedbed_entry();
    assert!(submit(&s, &e).is_err());
    assert!(s.get("garden-first").unwrap().is_none());
    assert!(notices(&s).is_empty());
    conn.execute_batch("DROP TRIGGER fail_seedbed;").unwrap();
    advance(&s, &mut e, "retry");
    assert_eq!(notices(&s).len(), 1);
    assert!(!s.commit_entry(&e, None).unwrap());
    assert_eq!(notices(&s).len(), 1);
    let output = dir.path().join("corrupt-seedbed");
    s.export(&output).unwrap();
    let path = output.join("story-seedbed.json");
    let mut document: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for step in [0, 5, 7] {
        document["deliveries"]["garden-first"]["step"] = json!(step);
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(LegacySnapshot::read(&output).is_err());
    }
}

#[test]
fn seedbed_full_notice_queue_defers_the_page_without_dropping_the_report() {
    let (dir, s, token, _) = fixture();
    let root = seedbed_thread(&s, &token);
    participate(&s, root, "garden-first");
    let now = now_ms();
    let queued: Vec<_> = (1..=2000).map(|id| json!({"id":id,"title":"Existing notice","text":"Existing text","targets":["garden-second"],"createdAt":iso(now),"expiresAt":iso(now+86400000)})).collect();
    let conn = Connection::open(dir.path().join("data.sqlite")).unwrap();
    conn.execute(
        "INSERT INTO legacy_documents(name,content) VALUES('announcements.json',?1)",
        [serde_json::to_vec(&json!({"nextId":2001,"announcements":queued})).unwrap()],
    )
    .unwrap();
    let mut e = seedbed_entry();
    advance(&s, &mut e, "queue-full");
    assert_eq!(
        s.get("garden-first").unwrap().unwrap()["reportRevision"],
        "queue-full"
    );
    assert_eq!(
        s.seedbed_story_status().unwrap()["progressCounts"],
        json!([0, 0, 0, 0, 0, 0])
    );
    s.revoke_announcement(1).unwrap();
    advance(&s, &mut e, "space-available");
    let sent = notices(&s);
    assert_eq!(sent.len(), 2000);
    assert_eq!(sent.last().unwrap()["id"], 2001);
    assert_eq!(sent.last().unwrap()["targets"], json!(["garden-first"]));
    assert_eq!(
        s.seedbed_story_status().unwrap()["progressCounts"],
        json!([1, 0, 0, 0, 0, 0])
    );
}
