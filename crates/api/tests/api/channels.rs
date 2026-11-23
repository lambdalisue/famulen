//! The channels the connected services give, and their calendars, through
//! the API.

use std::sync::Arc;

use famulen_api::Api;
use serde_json::{Value, json};

use crate::support::{self, DataDir, call, define, temp};

/// A service of the owner's own that the owner talks through, as a pair of
/// glasses would.
const GLASSES: &str = r#"
format = 1
id = "glasses"
name = "Glasses"

[account]
token = { label = "合言葉", secret = true }

[transport]
http = { base = "https://glasses.example" }

[receive.said]
destination = "channel"

[receive.said.http]
verify = { token = { value = "{header.X-Token}", equals = "{account.token}" } }
"#;

fn open(dir: &DataDir) -> Api {
    define(dir, "glasses", GLASSES);
    let api = support::open(dir);
    for name in ["channel", "lookout", "calendar"] {
        api.app()
            .gateway
            .register_destination(name, Arc::new(|_| {}));
    }
    api
}

#[tokio::test]
async fn a_service_added_with_a_receiver_for_the_channel_is_listed_as_a_channel_and_can_be_turned_off()
 {
    let dir = temp();
    let api = open(&dir);
    let added = call(
        &api,
        "ConnectionService/AddConnection",
        json!({"target": "glasses", "how": "definition", "env": "token=t-1"}),
    )
    .await;
    let id = added["connection"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("{added}"))
        .to_owned();

    let listed = call(&api, "ChannelService/ListChannels", json!({})).await;
    let off = call(
        &api,
        "ChannelService/SetChannelEnabled",
        json!({"connection": id, "receiver": "said", "enabled": false}),
    )
    .await;
    let detail = call(&api, "ConnectionService/GetConnection", json!({"id": id})).await;

    assert_eq!(listed["channels"][0]["connection"], json!(id), "{listed}");
    assert_eq!(listed["channels"][0]["receiver"], "said");
    assert_eq!(listed["channels"][0]["service"], "Glasses");
    assert_eq!(listed["channels"][0]["enabled"], json!(true));
    assert_eq!(
        listed["channels"][0]["state"],
        detail["connection"]["state"]
    );
    assert_eq!(
        detail["connection"]["transportKind"], "TRANSPORT_KIND_DEFINITION",
        "{detail}"
    );
    assert_eq!(
        off["channels"][0]["enabled"],
        Value::Null,
        "turned off, the channel is still listed: {off}"
    );
    assert_eq!(off["channels"][0]["receiver"], "said");
    assert_eq!(
        detail["receivers"][0]["destination"],
        Value::Null,
        "the receiver goes nowhere: {detail}"
    );
    assert_eq!(
        detail["connection"]["id"],
        json!(id),
        "the connection stays"
    );
}

#[tokio::test]
async fn a_receiver_that_is_not_a_channel_cannot_be_turned_on_as_one() {
    let dir = temp();
    let api = open(&dir);

    let refused = call(
        &api,
        "ChannelService/SetChannelEnabled",
        json!({"connection": "nothing", "receiver": "said", "enabled": true}),
    )
    .await;

    assert!(refused["code"].is_string(), "{refused}");
}

#[tokio::test]
async fn a_calendar_says_which_google_account_and_calendar_it_is_read_from() {
    let dir = temp();
    let api = open(&dir);

    let added = call(&api, "CalendarService/AddCalendar",
        json!({"name": "仕事", "googleConnection": "google-1", "googleCalendar": "team@example.com"}),
    )
    .await;
    let primary = call(
        &api,
        "CalendarService/AddCalendar",
        json!({"name": "わたし", "googleConnection": "google-1"}),
    )
    .await;

    assert_eq!(added["googleConnection"], "google-1", "{added}");
    assert_eq!(added["googleCalendar"], "team@example.com");
    assert_eq!(primary["googleCalendar"], "primary", "{primary}");
}
