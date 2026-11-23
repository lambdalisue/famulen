//! How long the Slack channel of Famulen attends a channel after a mention, as the
//! console sets it.

use serde_json::json;

use crate::support::{call, open, temp};

#[tokio::test]
async fn ten_minutes_until_the_owner_sets_another_length_and_zero_turns_it_off() {
    let dir = temp();
    let api = open(&dir);

    let before = call(&api, "ChannelService/ListWorkspaces", json!({})).await;
    let longer = call(
        &api,
        "ChannelService/SetAttention",
        json!({ "minutes": 20 }),
    )
    .await;
    let off = call(&api, "ChannelService/SetAttention", json!({ "minutes": 0 })).await;
    let after = call(&api, "ChannelService/ListWorkspaces", json!({})).await;

    assert_eq!(before["attentionMinutes"], 10);
    assert_eq!(longer["attentionMinutes"], 20);
    assert!(off.get("attentionMinutes").is_none_or(|m| m == 0), "{off}");
    assert!(
        after.get("attentionMinutes").is_none_or(|m| m == 0),
        "{after}"
    );
}
