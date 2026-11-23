//! The client against fake servers.

use serde_json::json;

use super::fake::{Era as FakeEra, FakeServer, stdio_server};
use super::*;

fn names(tools: &[Tool]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_str()).collect()
}

#[tokio::test]
async fn a_legacy_stdio_server_is_initialized_after_the_probe_fails() {
    let client = McpClient::connect(&stdio_server("legacy"), None)
        .await
        .unwrap();

    assert_eq!(client.era(), &Era::Legacy("2025-11-25".into()));
    client.close().await;
}

#[tokio::test]
async fn stdio_tools_are_listed_across_pages() {
    let client = McpClient::connect(&stdio_server("legacy"), None)
        .await
        .unwrap();

    let tools = client.list_tools().await.unwrap();

    assert_eq!(names(&tools), ["echo", "fail", "seen"]);
    assert_eq!(tools[0].read_only, Some(true));
    assert_eq!(tools[0].description.as_deref(), Some("Echo text"));
    assert_eq!(tools[1].read_only, None);
}

#[tokio::test]
async fn a_stdio_server_request_is_refused_and_notifications_are_ignored() {
    let client = McpClient::connect(&stdio_server("legacy"), None)
        .await
        .unwrap();
    client.list_tools().await.unwrap();

    let seen = client.call_tool("seen", json!({})).await.unwrap();

    assert_eq!(
        seen.text,
        "server/discover,initialize,notifications/initialized,tools/list,response:from-server:-32601,tools/list,response:from-server:-32601,tools/call"
    );
}

#[tokio::test]
async fn a_tool_that_fails_is_a_result_and_an_unknown_tool_is_an_error() {
    let client = McpClient::connect(&stdio_server("legacy"), None)
        .await
        .unwrap();

    let echoed = client
        .call_tool("echo", json!({"text": "こんにちは"}))
        .await
        .unwrap();
    let failed = client.call_tool("fail", json!({})).await.unwrap();
    let unknown = client.call_tool("nope", json!({})).await;

    assert_eq!(echoed.text, "こんにちは");
    assert!(!echoed.is_error);
    assert_eq!(failed.text, "it broke");
    assert!(failed.is_error);
    assert!(matches!(
        unknown,
        Err(McpError::Rpc(RpcError { code: -32602, .. }))
    ));
}

#[tokio::test]
async fn a_modern_stdio_server_is_spoken_to_without_a_handshake() {
    let client = McpClient::connect(&stdio_server("modern"), None)
        .await
        .unwrap();

    let tools = client.list_tools().await.unwrap();
    let seen = client.call_tool("seen", json!({})).await.unwrap();

    assert_eq!(client.era(), &Era::Modern(PROTOCOL_VERSION.into()));
    assert_eq!(client.instructions(), Some("Be kind."));
    assert_eq!(names(&tools), ["echo", "fail", "seen"]);
    assert_eq!(
        seen.text,
        "server/discover,tools/list,tools/list,tools/call"
    );
}

#[tokio::test]
async fn a_command_that_cannot_start_is_a_transport_error() {
    let config = ServerConfig::stdio("/nonexistent/famulen-mcp-server", Vec::<String>::new());

    let result = McpClient::connect(&config, None).await;

    assert!(matches!(result, Err(McpError::Transport(_))));
}

#[tokio::test]
async fn a_modern_http_server_gets_metadata_headers_and_sse_answers_are_read() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let client = McpClient::connect(&ServerConfig::http(fake.url()), None)
        .await
        .unwrap();

    let tools = client.list_tools().await.unwrap();

    assert_eq!(client.era(), &Era::Modern(PROTOCOL_VERSION.into()));
    assert_eq!(
        names(&tools),
        ["execute_sql", "echo"],
        "bad_header is left out"
    );
    let lists = fake.received_method("tools/list");
    assert_eq!(lists.len(), 2);
    assert_eq!(
        lists[0].header("accept"),
        Some("application/json, text/event-stream")
    );
    assert_eq!(lists[0].header("mcp-method"), Some("tools/list"));
    assert_eq!(lists[1].body["params"]["cursor"], json!("2"));
    assert!(
        fake.received()
            .iter()
            .all(|received| received.header("mcp-session-id").is_none())
    );
}

#[tokio::test]
async fn a_modern_http_tool_call_mirrors_annotated_params_into_headers() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let client = McpClient::connect(&ServerConfig::http(fake.url()), None)
        .await
        .unwrap();
    client.list_tools().await.unwrap();

    let result = client
        .call_tool(
            "execute_sql",
            json!({"region": "us-west1", "query": "SELECT 1"}),
        )
        .await
        .unwrap();

    assert_eq!(result.text, "ran in us-west1");
    let call = &fake.received_method("tools/call")[0];
    assert_eq!(call.header("mcp-name"), Some("execute_sql"));
    assert_eq!(call.header("mcp-param-region"), Some("us-west1"));
}

#[tokio::test]
async fn a_header_mismatch_lists_the_tools_again_and_retries_once() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let client = McpClient::connect(&ServerConfig::http(fake.url()), None)
        .await
        .unwrap();

    let result = client
        .call_tool("execute_sql", json!({"region": "eu-west1"}))
        .await
        .unwrap();

    assert_eq!(result.text, "ran in eu-west1");
    assert_eq!(fake.received_method("tools/call").len(), 2);
    assert_eq!(fake.received_method("tools/list").len(), 2);
}

#[tokio::test]
async fn a_legacy_http_server_keeps_a_session_and_the_negotiated_version() {
    let fake = FakeServer::start(FakeEra::Legacy, None).await;
    let client = McpClient::connect(&ServerConfig::http(fake.url()), None)
        .await
        .unwrap();

    let tools = client.list_tools().await.unwrap();
    let echoed = client
        .call_tool("echo", json!({"text": "hi"}))
        .await
        .unwrap();
    client.close().await;

    assert_eq!(names(&tools), ["echo"]);
    assert_eq!(echoed.text, "hi");
    let received = fake.received();
    let initialized = received
        .iter()
        .find(|received| received.method() == Some("notifications/initialized"))
        .unwrap();
    assert_eq!(initialized.header("mcp-session-id"), Some("session-1"));
    assert_eq!(
        initialized.header("mcp-protocol-version"),
        Some("2025-06-18")
    );
    let reply = received
        .iter()
        .find(|received| received.body.get("id") == Some(&json!("server-1")))
        .unwrap();
    assert_eq!(reply.body["error"]["code"], json!(-32601));
    let ended = received.last().unwrap();
    assert_eq!(ended.http_method, "DELETE");
    assert_eq!(ended.header("mcp-session-id"), Some("session-1"));
}

#[tokio::test]
async fn a_401_is_unauthorized_with_the_challenge_it_carried() {
    let fake = FakeServer::start(FakeEra::Modern, Some("access-1")).await;

    let result = McpClient::connect(&ServerConfig::http(fake.url()), None).await;

    let Err(McpError::Unauthorized {
        resource_metadata,
        scope,
    }) = result
    else {
        panic!("expected Unauthorized");
    };
    assert_eq!(
        resource_metadata,
        Some(format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            fake.base()
        ))
    );
    assert_eq!(scope.as_deref(), Some("files:read"));
}

#[tokio::test]
async fn a_bearer_token_is_sent_on_every_request() {
    let fake = FakeServer::start(FakeEra::Modern, Some("access-1")).await;

    let client = McpClient::connect(&ServerConfig::http(fake.url()), Some("access-1"))
        .await
        .unwrap();
    client.list_tools().await.unwrap();

    assert!(
        fake.received()
            .iter()
            .all(|received| received.header("authorization") == Some("Bearer access-1"))
    );
}

#[tokio::test]
async fn the_login_flow_discovers_registers_exchanges_and_refreshes() {
    let fake = FakeServer::start(FakeEra::Modern, Some("access-1")).await;
    let http = reqwest::Client::new();
    let redirect_uri = "https://famulen.example.com/oauth/callback";
    let Err(McpError::Unauthorized {
        resource_metadata,
        scope,
    }) = McpClient::connect(&ServerConfig::http(fake.url()), None).await
    else {
        panic!("expected Unauthorized");
    };

    let resource = discover_protected_resource(&http, fake.url(), resource_metadata.as_deref())
        .await
        .unwrap();
    let server = discover_auth_server(&http, &resource.authorization_servers[0])
        .await
        .unwrap();
    let client = register_client(&http, &server, redirect_uri, "Famulen")
        .await
        .unwrap();
    let pkce = Pkce::generate();
    let state = generate_state();
    let scope = select_scope(scope.as_deref(), &resource, &server);
    let url = authorization_url(
        &server,
        &AuthorizationRequest {
            client_id: &client.client_id,
            redirect_uri,
            pkce: &pkce,
            state: &state,
            resource: &resource.resource,
            scope: scope.as_deref(),
        },
    )
    .unwrap();
    *fake.challenge.lock().unwrap() = Some(pkce.challenge.clone());
    let code =
        authorization_code(&server, &state, format!("code=good-code&state={state}")).unwrap();
    let tokens = exchange_code(
        &http,
        &server,
        &client,
        &CodeExchange {
            code: &code,
            verifier: &pkce.verifier,
            redirect_uri,
            resource: &resource.resource,
        },
    )
    .await
    .unwrap();
    let refreshed = refresh_tokens(
        &http,
        &server,
        &client,
        tokens.refresh_token.as_deref().unwrap(),
        &resource.resource,
    )
    .await
    .unwrap();
    let mcp = McpClient::connect(&ServerConfig::http(fake.url()), Some(&tokens.access_token)).await;

    assert_eq!(resource.resource, fake.url());
    assert_eq!(server.issuer, fake.base());
    assert_eq!(client.client_id, "client-1");
    assert_eq!(
        fake.registrations.lock().unwrap()[0]["redirect_uris"],
        json!([redirect_uri])
    );
    assert_eq!(scope.as_deref(), Some("files:read offline_access"));
    assert!(
        url.as_str()
            .starts_with(&format!("{}/authorize?", fake.base()))
    );
    assert_eq!(tokens.access_token, "access-1");
    assert!(tokens.expires_at.is_some());
    let exchange = &fake.token_requests.lock().unwrap()[0];
    assert_eq!(exchange.get("resource"), Some(&fake.url()));
    assert_eq!(
        exchange.get("client_id").map(String::as_str),
        Some("client-1")
    );
    assert_eq!(refreshed.access_token, "access-2");
    assert_eq!(refreshed.refresh_token.as_deref(), Some("refresh-1"));
    assert!(mcp.is_ok());
}

#[tokio::test]
async fn a_wrong_verifier_is_rejected_by_the_token_endpoint() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let http = reqwest::Client::new();
    let server = discover_auth_server(&http, fake.base()).await.unwrap();
    let client = ClientRegistration {
        client_id: "client-1".into(),
        client_secret: None,
        token_endpoint_auth_method: None,
    };
    *fake.challenge.lock().unwrap() = Some(Pkce::from_verifier("right").challenge);

    let result = exchange_code(
        &http,
        &server,
        &client,
        &CodeExchange {
            code: "good-code",
            verifier: "wrong",
            redirect_uri: "https://famulen.example.com/cb",
            resource: &fake.url(),
        },
    )
    .await;

    assert_eq!(
        result,
        Err(OAuthError::Rejected {
            error: "invalid_grant".into(),
            description: Some("no".into())
        })
    );
}

#[tokio::test]
async fn protected_resource_metadata_is_found_at_the_well_known_path_without_a_hint() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;

    let resource = discover_protected_resource(&reqwest::Client::new(), fake.url(), None)
        .await
        .unwrap();

    assert_eq!(
        resource.authorization_servers,
        vec![fake.base().to_string()]
    );
    assert_eq!(resource.scopes_supported, Some(vec!["files:read".into()]));
}

#[tokio::test]
async fn protected_resource_metadata_for_another_server_is_refused() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let elsewhere = "http://127.0.0.1:9/mcp";

    let result = discover_protected_resource(
        &reqwest::Client::new(),
        elsewhere,
        Some(&format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            fake.base()
        )),
    )
    .await;

    assert!(matches!(result, Err(OAuthError::Discovery(_))));
}

#[tokio::test]
async fn an_issuer_with_a_path_falls_back_to_openid_configuration() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;
    let issuer = format!("{}/tenant", fake.base());

    let server = discover_auth_server(&reqwest::Client::new(), &issuer)
        .await
        .unwrap();

    assert_eq!(server.issuer, issuer);
}

#[tokio::test]
async fn metadata_naming_another_issuer_is_refused() {
    let fake = FakeServer::start(FakeEra::Modern, None).await;

    let result =
        discover_auth_server(&reqwest::Client::new(), format!("{}/impostor", fake.base())).await;

    assert!(matches!(result, Err(OAuthError::Discovery(_))));
}
