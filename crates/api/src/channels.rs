//! The channels other than the console, and the Slack workspaces the bot is in.

use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::slack::{attention, names};

use crate::connections::{state_of, status_label};
use crate::pb::*;
use crate::{Api, OrFail, actor, changed, fail, given};

impl Api {
    fn workspaces_view(&self) -> anyhow::Result<ListWorkspacesResponse> {
        let workspaces = self
            .app
            .workspaces()?
            .into_iter()
            .map(|w| Workspace {
                status: status_label(&w.status).into(),
                id: w.id,
                name: w.name,
                owner: w.owner,
                main: w.main,
                ..Default::default()
            })
            .collect();
        Ok(ListWorkspacesResponse {
            workspaces,
            attention_minutes: attention::minutes(&self.app.store)?,
            ..Default::default()
        })
    }

    fn channels_view(&self) -> anyhow::Result<ListChannelsResponse> {
        let channels = self
            .app
            .channels()?
            .into_iter()
            .map(|c| Channel {
                status: status_label(&c.status).into(),
                state: state_of(&c.status).into(),
                connection: c.connection,
                receiver: c.receiver,
                name: c.name,
                service: c.service,
                definition: c.definition,
                enabled: c.enabled,
                ..Default::default()
            })
            .collect();
        Ok(ListChannelsResponse {
            channels,
            ..Default::default()
        })
    }
}

/// Where the Slack app's names are changed: its App Home page when the
/// app's id is known.
fn slack_settings_url(app: Option<&str>) -> String {
    match app.and_then(given) {
        Some(app) => format!("https://api.slack.com/apps/{app}/app-home"),
        None => "https://api.slack.com/apps".into(),
    }
}

impl Api {
    async fn slack_name(&self, w: famulen_core::channel::Workspace) -> SlackName {
        let values = self.app.gateway.account_values(&w.id).unwrap_or_default();
        let mut name = SlackName {
            settings_url: slack_settings_url(values.get("app").map(String::as_str)),
            workspace: w.id.clone(),
            workspace_name: w.name,
            ..Default::default()
        };
        let read = async {
            let bot = values
                .get("bot_user")
                .ok_or_else(|| anyhow::anyhow!("the bot is not known yet"))?;
            let connection = self.need_connection(&w.id)?;
            let result = self
                .app
                .gateway
                .call_as_owner(&connection, "user_info", serde_json::json!({ "user": bot }))
                .await?;
            anyhow::ensure!(!result.is_error, "Slack answered: {}", result.text);
            Ok::<_, anyhow::Error>(serde_json::from_str::<serde_json::Value>(&result.text)?)
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), read).await {
            Ok(Ok(answer)) => (name.display_name, name.handle) = names::shown(&answer),
            Ok(Err(error)) => name.problem = error.to_string(),
            Err(_) => name.problem = "Slack did not answer in time".into(),
        }
        name
    }
}

impl ChannelService for Api {
    async fn list_channels(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListChannelsResponse> {
        Response::ok(self.channels_view().or_fail()?)
    }

    async fn set_channel_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetChannelEnabledRequest>,
    ) -> ServiceResult<ListChannelsResponse> {
        let r = request.to_owned_message();
        self.app
            .set_channel_enabled(actor(&ctx), &r.connection, &r.receiver, r.enabled)
            .or_fail()?;
        self.app.journal.record(
            "setting",
            format!(
                "channel {}/{} {}",
                r.connection,
                r.receiver,
                if r.enabled { "on" } else { "off" }
            ),
        );
        self.app.changed.notify_one();
        changed(self);
        Response::ok(self.channels_view().or_fail()?)
    }

    async fn list_slack_names(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListSlackNamesResponse> {
        let workspaces = self.app.workspaces().or_fail()?;
        let names =
            futures::future::join_all(workspaces.into_iter().map(|w| self.slack_name(w))).await;
        Response::ok(ListSlackNamesResponse {
            names,
            ..Default::default()
        })
    }

    async fn list_workspaces(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListWorkspacesResponse> {
        Response::ok(self.workspaces_view().or_fail()?)
    }

    async fn set_attention(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetAttentionRequest>,
    ) -> ServiceResult<ListWorkspacesResponse> {
        let minutes = request.to_owned_message().minutes;
        attention::set_minutes(&self.app.store, minutes).or_fail()?;
        Response::ok(self.workspaces_view().or_fail()?)
    }

    async fn set_main_workspace(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        self.app.set_main_workspace(request.id).or_fail()?;
        self.app.changed.notify_one();
        Response::ok(Empty::default())
    }

    async fn remove_workspace(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        let id = request.id.to_owned();
        if !self.app.workspaces().or_fail()?.iter().any(|w| w.id == id) {
            return Err(fail(format!("no workspace {id}")));
        }
        self.remove_connection(&ctx, &id).or_fail()?;
        self.app
            .journal
            .record("setting", format!("Slack workspace {id} removed"));
        Response::ok(Empty::default())
    }

    async fn send_slack(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SendSlackRequest>,
    ) -> ServiceResult<SendSlackResponse> {
        use famulen_core::slack::reply::{self, Posted};
        let r = request.to_owned_message();
        let clients = self
            .app
            .slack
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| {
                fail("no Slack workspace is connected; answer on the console with post_message")
            })?;
        if famulen_core::told::is_over(&self.app).or_fail()? {
            return Err(fail(
                "what you were told of was settled elsewhere meanwhile; say nothing of it",
            ));
        }
        let client = clients.get(given(&r.workspace)).or_fail()?;
        let workspace = given(&r.workspace).unwrap_or(clients.main_id());
        // The secretary answering in a channel it attends keeps attending it.
        attention::extend(&self.app.store, workspace, &r.channel, chrono::Utc::now()).or_fail()?;
        let Posted { ts, text } = reply::send(
            client,
            &self.app.store,
            workspace,
            &r.channel,
            &r.text,
            Some(r.thread_ts.clone()),
        )
        .await
        .or_fail()?;
        famulen_core::told::said(&self.app, workspace, &r.channel, &ts, &text).or_fail()?;
        let meta = serde_json::json!({
            "origin": format!("Slack · {}", self.app.workspace_label(workspace)),
            "workgroup": self.app.workgroups.working().or_fail()?,
        });
        self.app
            .console
            .add(famulen_core::console_log::Who::Aide, &r.text, meta)
            .or_fail()?;
        Response::ok(SendSlackResponse {
            ts,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slack_names_are_changed_on_the_apps_home_page() {
        assert_eq!(
            slack_settings_url(Some("A01")),
            "https://api.slack.com/apps/A01/app-home"
        );
        assert_eq!(slack_settings_url(None), "https://api.slack.com/apps");
    }
}
