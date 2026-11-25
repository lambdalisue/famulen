//! The owner's accounts on outside services.

use std::collections::BTreeMap;

use connectrpc::{ConnectError, RequestContext, Response, ServiceRequest, ServiceResult};
use famulen_core::catalog;
use famulen_core::connections::{self as core, Secret, Transport, guess_kind, words};
use famulen_core::gateway::{Call, GATEWAY};
use famulen_core::workgroups::MemberKind;

use crate::definitions::login_how;
use crate::pb::*;
use crate::permissions::permission_from;
use crate::{Api, OrFail, actor, changed, fail, given, unless_held};

/// The status of a connection that stands well.
const CONNECTED: &str = "つながっている";

/// How a connection stands, as the console says it.
pub(crate) fn status_label(status: &str) -> &str {
    given(status).unwrap_or("ログイン待ち")
}

pub(crate) fn state_of(status: &str) -> ConnectionState {
    match status.trim() {
        CONNECTED => ConnectionState::CONNECTION_STATE_CONNECTED,
        "" => ConnectionState::CONNECTION_STATE_LOGIN_PENDING,
        _ => ConnectionState::CONNECTION_STATE_TROUBLED,
    }
}

fn transport_kind(transport: &Transport) -> TransportKind {
    match transport {
        Transport::McpRemote { .. } => TransportKind::TRANSPORT_KIND_MCP_REMOTE,
        Transport::McpLocal { .. } => TransportKind::TRANSPORT_KIND_MCP_LOCAL,
        Transport::Api { .. } => TransportKind::TRANSPORT_KIND_API,
        Transport::Defined { .. } => TransportKind::TRANSPORT_KIND_DEFINITION,
    }
}

pub(crate) fn is_slack(c: &core::Connection) -> bool {
    matches!(&c.transport, Transport::Defined { definition, .. } if definition == famulen_core::channel::DEFINITION)
}

/// `KEY=VALUE` lines as a map.
fn lines(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .filter(|(k, _)| !k.is_empty())
        .collect()
}

pub(crate) fn connection(c: &core::Connection) -> Connection {
    let definition = match &c.transport {
        Transport::Defined { definition, .. } => definition.as_str(),
        _ => "",
    };
    let watching = match definition {
        famulen_core::arrivals::GOOGLE => "メールとカレンダーを読んでいる",
        famulen_core::channel::DEFINITION => "あなたへの頼みと、ほかの発言を受けている",
        _ => "",
    };
    Connection {
        id: c.id.clone(),
        name: c.name.clone(),
        kind: c.kind.clone(),
        account: c.account.clone(),
        transport: c.transport.label().into(),
        status: status_label(&c.status).into(),
        state: state_of(&c.status).into(),
        transport_kind: transport_kind(&c.transport).into(),
        ready: c.secret.holds("bearer")
            || c.secret.holds("api_key")
            || (!definition.is_empty() && c.status == CONNECTED),
        watching: watching.into(),
        ..Default::default()
    }
}

impl Api {
    pub(crate) fn need_connection(&self, id: &str) -> anyhow::Result<core::Connection> {
        self.app
            .connections
            .get(id)?
            .ok_or_else(|| anyhow::anyhow!("no connection {id}"))
    }

    /// A connection could not be set up: it is not left behind, and the
    /// caller reads why.
    fn undo_add(&self, id: &str, error: anyhow::Error) -> ConnectError {
        match self.app.gateway.remove_connection(GATEWAY, id).or_fail() {
            Ok(_) => fail(format!("{error:#}")),
            Err(left) => left,
        }
    }

    /// Remove a connection, with its place in every workgroup.
    pub(crate) fn remove_connection(&self, ctx: &RequestContext, id: &str) -> anyhow::Result<()> {
        self.app.gateway.remove_connection(actor(ctx), id)?;
        self.app
            .workgroups
            .forget_member(MemberKind::Connection, id)?;
        famulen_core::runtime::sync_receivers(&self.app)?;
        self.app.changed.notify_one();
        changed(self);
        Ok(())
    }

    /// Add a connection made from the service definition `definition`, with
    /// the account's values given as `KEY=VALUE` lines, and start its login
    /// when the definition's login is OAuth 2.0.
    async fn add_defined(
        &self,
        definition: &str,
        values: &str,
        key: &str,
        origin: &str,
    ) -> ServiceResult<ConnectResult> {
        let values = lines(values);
        let c = self
            .app
            .gateway
            .add_defined(definition, values, None)
            .or_fail()?;
        self.app.journal.record(
            "setting",
            format!("service {} added from {definition}", c.id),
        );
        let login = self
            .app
            .gateway
            .definitions()
            .get(definition)
            .and_then(|d| d.login.clone());
        let oauth = login.as_ref().is_some_and(|l| l.oauth2.is_some());
        let kept = match login {
            Some(l) if l.key.is_some() && !key.trim().is_empty() => {
                self.app.gateway.set_api_key(&c.id, key, "")
            }
            Some(l) if l.session.is_some() => self.app.gateway.open_session(&c.id).await,
            _ => Ok(()),
        };
        if let Err(error) = kept {
            return Err(self.undo_add(&c.id, error));
        }
        let login_url = if oauth {
            // A login that cannot start leaves no connection behind.
            let begun = self
                .callback(origin)
                .and_then(|callback| self.app.gateway.begin_defined_login(&c.id, &callback));
            match begun {
                Ok(url) => url,
                Err(error) => return Err(self.undo_add(&c.id, error)),
            }
        } else {
            let _ = self.app.gateway.read_tools(&c.id).await;
            famulen_core::runtime::sync_receivers(&self.app).or_fail()?;
            String::new()
        };
        self.app.changed.notify_one();
        changed(self);
        let c = self.app.connections.get(&c.id).or_fail()?.unwrap_or(c);
        Response::ok(ConnectResult {
            connection: connection(&c).into(),
            login_url,
            ..Default::default()
        })
    }

    /// Where a login comes back to: the configured console URL, or the
    /// origin the console was opened at.
    fn callback(&self, origin: &str) -> anyhow::Result<String> {
        let base = self
            .app
            .console_url
            .clone()
            .or_else(|| Some(origin.trim_end_matches('/').to_owned()).filter(|o| !o.is_empty()))
            .ok_or_else(|| {
                anyhow::anyhow!("set FAMULEN_CONSOLE_URL to where the console is opened")
            })?;
        Ok(format!("{base}/oauth/callback"))
    }

    fn connection_detail(&self, id: &str) -> anyhow::Result<ConnectionDetail> {
        if id == famulen_core::hub::SERVER_NAME {
            let label = format!("{} 自身の道具", self.app.aide_name());
            let scope = format!("conn:{id}");
            // Own tools are stored with `read_only` meaning "runs without asking",
            // which the permission defaults read; what each one does and whether
            // it only reads come from the API's definition.
            let own: std::collections::HashMap<String, (String, bool)> =
                crate::tools::tools_from(crate::DESCRIPTOR_SET)?
                    .into_iter()
                    .map(|t| (t.def.name, (t.def.description, t.def.read_only)))
                    .collect();
            let tools = self
                .scope_tools(&scope)?
                .into_iter()
                .map(|(name, read_only)| match own.get(&name) {
                    Some((description, reads)) => self.tool_setting(id, &name, description, *reads),
                    None => self.tool_setting(id, &name, "", read_only),
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            return Ok(ConnectionDetail {
                connection: Connection {
                    id: id.to_owned(),
                    name: label,
                    kind: id.to_owned(),
                    transport: "組み込み".into(),
                    status: CONNECTED.into(),
                    ready: true,
                    ..Default::default()
                }
                .into(),
                tools,
                ..Default::default()
            });
        }
        let c = self.need_connection(id)?;
        let tools = self
            .app
            .connections
            .tools(id)?
            .into_iter()
            .map(|t| self.tool_setting(id, &t.name, &t.description, t.read_only == Some(true)))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let target = match &c.transport {
            Transport::McpRemote { url } => url.clone(),
            Transport::McpLocal { command, args, .. } => std::iter::once(command.clone())
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
            Transport::Api { base_url, .. } => base_url.clone(),
            Transport::Defined { definition, .. } => definition.clone(),
        };
        let has_secret = c.secret.holds("api_key") || c.secret.holds("bearer");
        let (receivers, destinations, values) = match &c.transport {
            Transport::Defined { definition, .. } => self.defined_detail(id, definition)?,
            _ => Default::default(),
        };
        let login = match &c.transport {
            Transport::McpRemote { .. } => "mcp",
            Transport::Defined { definition, .. } => self
                .app
                .gateway
                .definitions()
                .get(definition)
                .map(login_how)
                .filter(|how| !matches!(*how, "key" | "session"))
                .unwrap_or_default(),
            _ => "",
        };
        Ok(ConnectionDetail {
            connection: connection(&c).into(),
            tools,
            target,
            has_secret,
            receivers,
            destinations,
            values,
            login: login.into(),
            ..Default::default()
        })
    }

    /// Add a connection of a known service and start its login.
    async fn connect_known(
        &self,
        known: &catalog::Known,
        given: &BTreeMap<String, String>,
        origin: &str,
    ) -> anyhow::Result<ConnectResult> {
        let (c, login_url, added) = self
            .app
            .gateway
            .connect_known(known, given, &self.callback(origin)?)
            .await?;
        if added {
            self.app.journal.record(
                "setting",
                format!("service {} added from the known list", c.id),
            );
        }
        Ok(ConnectResult {
            connection: connection(&c).into(),
            login_url,
            ..Default::default()
        })
    }
}

impl ConnectionService for Api {
    async fn list_connections(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListConnectionsResponse> {
        let connections = self
            .app
            .connections
            .list()
            .or_fail()?
            .iter()
            .map(connection)
            .collect();
        Response::ok(ListConnectionsResponse {
            connections,
            ..Default::default()
        })
    }

    async fn get_connection(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<ConnectionDetail> {
        Response::ok(self.connection_detail(request.id).or_fail()?)
    }

    async fn get_kind(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<KindDetail> {
        Response::ok(self.kind_detail(request.id).or_fail()?)
    }

    async fn list_known(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListKnownResponse> {
        let definitions = self.app.gateway.definitions();
        let mut services: Vec<KnownService> = definitions
            .list()
            .filter(|d| d.app_spec.is_some())
            .map(|d| {
                let unavailable = self.app.gateway.app_values(d).err().unwrap_or_default();
                KnownService {
                    id: d.id.clone(),
                    name: d.name.clone(),
                    icon: format!("brand-{}", d.id),
                    what: d.about.clone().unwrap_or_default(),
                    available: unavailable.is_empty(),
                    unavailable,
                    definition: true,
                    ..Default::default()
                }
            })
            .collect();
        services.extend(catalog::KNOWN.iter().map(|k| {
            KnownService {
                id: k.id.into(),
                name: k.name.into(),
                icon: k.icon.into(),
                what: k.what.into(),
                available: true,
                fields: k
                    .asks
                    .iter()
                    .map(|a| AccountField {
                        name: a.name.into(),
                        label: a.label.into(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            }
        }));
        Response::ok(ListKnownResponse {
            services,
            ..Default::default()
        })
    }

    async fn connect_known(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ConnectKnownRequest>,
    ) -> ServiceResult<ConnectResult> {
        let r = request.to_owned_message();
        let known = catalog::find(&r.service)
            .ok_or_else(|| fail(format!("no known service {}", r.service)))?;
        Response::ok(
            self.connect_known(known, &lines(&r.env), &r.console_origin)
                .await
                .or_fail()?,
        )
    }

    async fn add_connection(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, AddConnectionRequest>,
    ) -> ServiceResult<ConnectResult> {
        let r = request.to_owned_message();
        let target = r.target.trim();
        if target.is_empty() {
            return Err(fail("give an MCP URL or a command"));
        }
        if r.how == "definition" {
            return self
                .add_defined(target, &r.env, &r.api_key, &r.console_origin)
                .await;
        }
        let remote = target.starts_with("https://") || target.starts_with("http://");
        let env = lines(&r.env);
        let transport = if r.how == "api" {
            Transport::Api {
                base_url: target.to_owned(),
                openapi_url: r.openapi.trim().to_owned(),
            }
        } else if remote {
            Transport::McpRemote {
                url: target.to_owned(),
            }
        } else {
            let mut parts = words(target).into_iter();
            let command = parts
                .next()
                .ok_or_else(|| fail("a local server needs a command"))?;
            Transport::McpLocal {
                command,
                args: parts.collect(),
                env: Default::default(),
            }
        };
        let kind = guess_kind(target);
        let id = self.app.gateway.fresh_id(&kind).or_fail()?;
        let mut name = r.name.trim().to_owned();
        if name.is_empty() {
            name.clone_from(&kind);
            if let Some(first) = name.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
        }
        let local = matches!(transport, Transport::McpLocal { .. });
        let keyed = !local && !r.api_key.trim().is_empty();
        let c = self
            .app
            .connections
            .add(&id, &name, &kind, transport, Secret::default(), "")
            .or_fail()?;
        self.app
            .journal
            .record("setting", format!("service {id} added"));
        // The owner's key or environment could not be kept: a connection
        // that would never work is not left behind.
        let kept = if keyed {
            self.app.gateway.set_api_key(&id, &r.api_key, &r.header)
        } else if local && !env.is_empty() {
            self.app.gateway.set_environment(&id, env)
        } else {
            Ok(())
        };
        if let Err(error) = kept {
            return Err(self.undo_add(&id, error));
        }
        let login_url = match self.app.gateway.read_tools(&id).await {
            Ok(_) => {
                self.app.changed.notify_one();
                String::new()
            }
            // A remote server that wants a login goes straight to it.
            Err(_) if remote && r.how != "api" && !keyed => self
                .app
                .gateway
                .begin_login(&id, &self.callback(&r.console_origin).or_fail()?)
                .await
                .unwrap_or_default(),
            Err(_) => String::new(),
        };
        let c = self.app.connections.get(&id).or_fail()?.unwrap_or(c);
        changed(self);
        Response::ok(ConnectResult {
            connection: connection(&c).into(),
            login_url,
            ..Default::default()
        })
    }

    async fn add_keyless_connection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AddKeylessConnectionRequest>,
    ) -> ServiceResult<Connection> {
        let r = request.to_owned_message();
        let c = self
            .app
            .gateway
            .add_keyless(
                actor(&ctx),
                &r.definition,
                r.values.into_iter().collect(),
                Some(&r.name),
            )
            .or_fail()?;
        if let Err(error) = self.app.gateway.read_tools(&c.id).await {
            return Err(self.undo_add(&c.id, error));
        }
        self.app.journal.record(
            "setting",
            format!("service {} added from {}", c.id, r.definition.trim()),
        );
        self.app.changed.notify_one();
        changed(self);
        Response::ok(connection(&self.need_connection(&c.id).or_fail()?))
    }

    async fn start_login(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, StartLoginRequest>,
    ) -> ServiceResult<GoTo> {
        let r = request.to_owned_message();
        let callback = self.callback(&r.console_origin).or_fail()?;
        let defined = self
            .app
            .connections
            .get(&r.id)
            .or_fail()?
            .is_some_and(|c| matches!(c.transport, Transport::Defined { .. }));
        let url = if defined {
            self.app
                .gateway
                .begin_defined_login(&r.id, &callback)
                .or_fail()?
        } else {
            self.app
                .gateway
                .begin_login(&r.id, &callback)
                .await
                .or_fail()?
        };
        Response::ok(GoTo {
            url,
            ..Default::default()
        })
    }

    async fn finish_login(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, FinishLoginRequest>,
    ) -> ServiceResult<LoginFinished> {
        let query = request.query.to_owned();
        let (id, now) = self.app.gateway.finish_login(&query).await.or_fail()?;
        if now {
            let _ = self.app.gateway.read_tools(&id).await;
            self.app.adopt_account(&id).or_fail()?;
            self.app
                .journal
                .record("setting", format!("service {id} logged in"));
            self.app.slack_looks.installed(&id);
            // Mail that comes from here on is read, not only what comes after
            // the next regular read.
            let c = self.need_connection(&id).or_fail()?;
            if famulen_core::arrivals::is_google(&c)
                && let Err(error) = self.app.read_mail(&id).await
            {
                tracing::warn!(%error, connection = %id, "could not mark where to read the mail from");
            }
            // The account is connected whether or not it could be greeted.
            if let Err(error) = self.app.greet(&id).await {
                tracing::warn!(%error, connection = %id, "cannot greet the owner");
            }
        }
        self.app.changed.notify_one();
        changed(self);
        let c = self.need_connection(&id).or_fail()?;
        let slack = is_slack(&c);
        Response::ok(LoginFinished {
            kind: if slack { "workspace" } else { "connection" }.into(),
            name: if c.account.is_empty() {
                c.name
            } else {
                c.account
            },
            id,
            ..Default::default()
        })
    }

    async fn edit_connection(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, EditConnectionRequest>,
    ) -> ServiceResult<Connection> {
        let r = request.to_owned_message();
        self.app
            .connections
            .update(
                &r.id,
                r.name.as_deref(),
                r.kind.as_deref(),
                r.account.as_deref(),
            )
            .or_fail()?;
        let c = self.need_connection(&r.id).or_fail()?;
        changed(self);
        Response::ok(connection(&c))
    }

    async fn set_secret(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, SetSecretRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let c = self.need_connection(&r.id).or_fail()?;
        self.app
            .gateway
            .set_api_key(&c.id, &r.api_key, &r.header)
            .or_fail()?;
        let _ = self.app.gateway.read_tools(&r.id).await;
        self.app.changed.notify_one();
        Response::ok(Empty::default())
    }

    async fn refresh_tools(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<ConnectionDetail> {
        let id = request.id.to_owned();
        self.app.gateway.read_tools(&id).await.or_fail()?;
        self.app.changed.notify_one();
        Response::ok(self.connection_detail(&id).or_fail()?)
    }

    async fn delete_connection(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        self.remove_connection(&ctx, request.id).or_fail()?;
        self.app
            .journal
            .record("setting", format!("service {} removed", request.id));
        Response::ok(Empty::default())
    }

    async fn set_permission(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPermissionRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let chosen = r.permission.as_known().and_then(permission_from);
        self.app
            .permissions
            .set_scoped(actor(&ctx), &r.scope, &r.tool, chosen)
            .or_fail()?;
        self.app.journal.record(
            "setting",
            format!(
                "permission {} {} = {}",
                r.scope,
                r.tool,
                chosen.map(|r| r.as_str()).unwrap_or("inherit")
            ),
        );
        Response::ok(Empty::default())
    }

    async fn set_permissions(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetPermissionsRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let chosen = r.permission.as_known().and_then(permission_from);
        for (tool, reads) in self.scope_tools(&r.scope).or_fail()? {
            let picked = match r.which.as_str() {
                "read" => reads,
                "write" => !reads,
                _ => true,
            };
            if picked {
                self.app
                    .permissions
                    .set_scoped(actor(&ctx), &r.scope, &tool, chosen)
                    .or_fail()?;
            }
        }
        self.app.journal.record(
            "setting",
            format!("permissions for {} {} changed", r.scope, r.which),
        );
        Response::ok(Empty::default())
    }

    async fn call_api(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CallApiRequest>,
    ) -> ServiceResult<CallApiResponse> {
        let r = request.to_owned_message();
        let c = self.need_connection(&r.connection).or_fail()?;
        if !matches!(c.transport, Transport::Api { .. }) {
            return Err(fail(format!("{} is not an API connection", c.id)));
        }
        let mut input = serde_json::Map::from_iter([("path".to_owned(), r.path.into())]);
        for (name, text) in [("query", &r.query), ("body", &r.body)] {
            if let Some(text) = given(text) {
                let value = serde_json::from_str(text).map_err(fail)?;
                input.insert(name.to_owned(), value);
            }
        }
        // The gateway alone holds the connection's key and judges its tools.
        let tool = format!("http_{}", r.method.to_ascii_lowercase());
        let actor = actor(&ctx);
        let origin = match actor {
            crate::SECRETARY => self.app.store.call_origin().or_fail()?,
            _ => None,
        };
        let call = Call {
            origin,
            ..Call::new(actor, &c.id, &tool, input.into())
        };
        let answer = unless_held(self.app.gateway.call(&call).await.or_fail()?)?;
        if answer.is_error {
            return Err(fail(answer.text));
        }
        Response::ok(CallApiResponse {
            body: answer.text,
            ..Default::default()
        })
    }

    async fn list_builtin_tools(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListBuiltinToolsResponse> {
        Response::ok(ListBuiltinToolsResponse {
            tools: self.builtin_tools().or_fail()?,
            ..Default::default()
        })
    }

    async fn list_definitions(
        &self,
        _ctx: RequestContext,
        _request: ServiceRequest<'_, Empty>,
    ) -> ServiceResult<ListDefinitionsResponse> {
        Response::ok(self.definition_list())
    }

    async fn get_definition_text(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<DefinitionText> {
        Response::ok(self.definition_text(request.id).or_fail()?)
    }

    async fn propose_definition_change(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ProposeDefinitionChangeRequest>,
    ) -> ServiceResult<Empty> {
        let r = request.to_owned_message();
        let outcome = self
            .app
            .gateway
            .propose_definition(actor(&ctx), &r.definition, &r.toml, &r.why)
            .await
            .or_fail()?;
        unless_held(outcome)?;
        Response::ok(Empty::default())
    }

    async fn list_definition_changes(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ListDefinitionChangesRequest>,
    ) -> ServiceResult<ListDefinitionChangesResponse> {
        Response::ok(self.definition_changes(request.definition).or_fail()?)
    }

    async fn revert_definition(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ByName>,
    ) -> ServiceResult<Empty> {
        self.app
            .gateway
            .revert_definition(actor(&ctx), request.id)
            .await
            .or_fail()?;
        Response::ok(Empty::default())
    }

    async fn set_destination(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetDestinationRequest>,
    ) -> ServiceResult<ConnectionDetail> {
        let r = request.to_owned_message();
        let destination = given(&r.destination);
        if let Some(d) = destination
            && !self.app.gateway.destination_names().iter().any(|n| n == d)
        {
            return Err(fail(format!("no destination {d}")));
        }
        self.app
            .gateway
            .set_destination(actor(&ctx), &r.connection, &r.receiver, destination)
            .or_fail()?;
        famulen_core::runtime::sync_receivers(&self.app).or_fail()?;
        changed(self);
        Response::ok(self.connection_detail(&r.connection).or_fail()?)
    }

    async fn set_account_value(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetAccountValueRequest>,
    ) -> ServiceResult<ConnectionDetail> {
        let r = request.to_owned_message();
        self.app
            .gateway
            .set_account_value(actor(&ctx), &r.connection, &r.name, &r.value)
            .await
            .or_fail()?;
        self.app.changed.notify_one();
        changed(self);
        Response::ok(self.connection_detail(&r.connection).or_fail()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_stands_well_waits_for_its_login_or_is_in_trouble() {
        for (status, state) in [
            (
                "つながっている",
                ConnectionState::CONNECTION_STATE_CONNECTED,
            ),
            ("", ConnectionState::CONNECTION_STATE_LOGIN_PENDING),
            ("ログインし直す", ConnectionState::CONNECTION_STATE_TROUBLED),
            (
                "つながらない（x）",
                ConnectionState::CONNECTION_STATE_TROUBLED,
            ),
        ] {
            assert_eq!(state_of(status), state, "{status}");
        }
    }
}
