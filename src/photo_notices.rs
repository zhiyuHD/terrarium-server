//! Compatibility fallback: photo URLs reach existing clients through private protocol receipts.
use crate::{
    policy::{at, iso},
    store::{Store, document, save_document},
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

const MAX_PER_BOX: usize = 32;
const MAX_PENDING: usize = crate::photo_cache::MAX_PHOTOS;

pub(crate) fn validate(client: &Value) -> Result<()> {
    let Some(notices) = client.get("photoReplies") else {
        return Ok(());
    };
    let notices = notices
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Invalid photo receipt queue"))?;
    ensure!(notices.len() <= MAX_PER_BOX, "Too many photo receipts");
    let mut previous = 0;
    for notice in notices {
        let id = notice["id"].as_u64().unwrap_or(0);
        ensure!(
            id > previous
                && id < 9_007_199_254_740_991
                && notice["title"] == "拍照链接回执"
                && notice["text"].as_str().is_some_and(
                    |s| s.starts_with("[photo-result]") && s.encode_utf16().count() <= 2000
                )
                && notice["targets"].as_array().is_some_and(
                    |a| a.len() == 1 && a[0].as_str().is_some_and(crate::protocol::valid_id)
                )
                && at(&notice["createdAt"])
                    .zip(at(&notice["expiresAt"]))
                    .is_some_and(|(a, b)| b - a == crate::photo_cache::PHOTO_TTL_MS),
            "Invalid photo receipt"
        );
        previous = id;
    }
    Ok(())
}
pub(crate) fn prune(client: &mut Value, now: i64, after: Option<u64>) -> bool {
    let Some(notices) = client.get_mut("photoReplies").and_then(Value::as_array_mut) else {
        return false;
    };
    let before = notices.len();
    notices.retain(|n| {
        at(&n["expiresAt"]).is_some_and(|at| at > now)
            && after.is_none_or(|a| n["id"].as_u64().unwrap_or(0) > a)
    });
    notices.len() != before
}
pub(crate) fn notices(all: &Value, id: &str, after: u64, now: i64) -> Vec<Value> {
    all[id]["photoReplies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| {
            n["id"].as_u64().unwrap_or(0) > after && at(&n["expiresAt"]).is_some_and(|at| at > now)
        })
        .cloned()
        .collect()
}

impl Store {
    pub fn photo_receipt(&self, box_id: &str, data: &Value, now: i64) -> Result<()> {
        self.transaction(|tx| {
            let mut all=document(tx,"protocol-notices.json")?.unwrap_or_else(||json!({}));
            for client in all.as_object_mut().unwrap().values_mut() { prune(client,now,None); }
            let total:usize=all.as_object().unwrap().values().map(|c|c["photoReplies"].as_array().map_or(0,Vec::len)).sum();
            ensure!(total<MAX_PENDING,"Read pending photo receipts before taking more photos");
            let client=all.as_object_mut().unwrap().entry(box_id.to_owned()).or_insert_with(||json!({"transport":"http"}));
            if client.get("photoReplies").is_none() {client["photoReplies"]=json!([]);}
            let replies=client["photoReplies"].as_array_mut().unwrap();
            ensure!(replies.len()<MAX_PER_BOX,"Read pending photo receipts before taking more photos");
            let first=crate::forum_bridge::allocate(tx,1)?;
            let mut data=data.clone();data["purpose"]=json!("备用发图地址：工具只接受 HTTPS 图片时可使用；24 小时后失效。按当前对话需要使用，不要求主动联系主人。");
            let notice=json!({"id":first,"title":"拍照链接回执","text":format!("[photo-result]{}",data),"targets":[box_id],"createdAt":iso(now),"expiresAt":data["expiresAt"]});
            ensure!(notice["text"].as_str().unwrap().encode_utf16().count()<=2000,"Photo receipt too large");
            replies.push(notice);save_document(tx,"protocol-notices.json",&all)?;
            Ok(())
        })
    }
    pub fn cleanup_photo_receipts(&self, now: i64) -> Result<()> {
        self.transaction(|tx| {
            let Some(mut all) = document(tx, "protocol-notices.json")? else {
                return Ok(());
            };
            let mut changed = false;
            for client in all.as_object_mut().unwrap().values_mut() {
                changed |= prune(client, now, None);
            }
            if changed {
                save_document(tx, "protocol-notices.json", &all)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    #[test]
    fn private_receipts_expire_without_polling_export_and_keep_operator_announcements_empty() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("data.sqlite")).unwrap();
        let now = 1_800_000_000_000;
        let data = json!({"url":"https://photos.example.test/image.png","expiresAt":iso(now+crate::photo_cache::PHOTO_TTL_MS),"stateHash":"a".repeat(64)});
        store.photo_receipt("garden-one", &data, now).unwrap();
        let page = store
            .announcement_page(Some("garden-one"), 0, false, None, now)
            .unwrap();
        assert_eq!(page["announcements"].as_array().unwrap().len(), 1);
        assert!(
            store
                .announcement_page(Some("garden-two"), 0, false, None, now)
                .unwrap()["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            store.list_announcements().unwrap()["announcements"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let snapshot = dir.path().join("export");
        store.export(&snapshot).unwrap();
        let restored = Store::open(dir.path().join("restored.sqlite")).unwrap();
        restored
            .import(crate::store::LegacySnapshot::read(&snapshot).unwrap())
            .unwrap();
        assert_eq!(
            restored
                .announcement_page(Some("garden-one"), 0, false, None, now)
                .unwrap(),
            page
        );
        store
            .cleanup_photo_receipts(now + crate::photo_cache::PHOTO_TTL_MS)
            .unwrap();
        assert!(
            store.observed_client("garden-one").unwrap()["photoReplies"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn limits_unread_receipts_and_keeps_the_cursor_unchanged_on_rejection() {
        let dir = TempDir::new().unwrap();
        let store = Store::open(dir.path().join("data.sqlite")).unwrap();
        let now = 1_800_000_000_000;
        let data = json!({"url":"https://photos.example.test/image.png","expiresAt":iso(now+crate::photo_cache::PHOTO_TTL_MS)});
        for _ in 0..MAX_PER_BOX {
            store.photo_receipt("garden-one", &data, now).unwrap();
        }
        let before = store
            .announcement_page(Some("garden-one"), 0, false, None, now)
            .unwrap();
        assert!(store.photo_receipt("garden-one", &data, now).is_err());
        assert_eq!(
            store
                .announcement_page(Some("garden-one"), 0, false, None, now)
                .unwrap(),
            before
        );
    }
}
