use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::models::Price;

fn control_success(response: &str) -> String {
    format!(
        r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"famulen","response":{response}}}}}"#
    )
}

/// Cut down from what Claude Code 2.1.331 answers for a Max account.
fn claude_answer() -> String {
    control_success(
        r#"{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":3,"resets_at":"2026-10-19T19:19:59.661431+00:00"},"limits":[{"kind":"session","group":"session","percent":3,"severity":"normal","resets_at":"2026-10-19T19:19:59.661431+00:00","scope":null,"is_active":false},{"kind":"weekly_all","group":"weekly","percent":86,"severity":"warning","resets_at":"2026-10-22T13:59:59.661454+00:00","scope":null,"is_active":true},{"kind":"weekly_scoped","group":"weekly","percent":0,"severity":"normal","resets_at":"2026-10-22T14:00:00+00:00","scope":{"model":{"id":null,"display_name":"Fable"},"surface":null},"is_active":false}]}}"#,
    )
}

fn limits_with(limits: &str) -> String {
    control_success(&format!(r#"{{"rate_limits":{{"limits":{limits}}}}}"#))
}

fn limits_of(line: &str) -> Vec<Limit> {
    match claude_reply(line) {
        Reply::Limits(list) => list,
        other => panic!("no limits: {other:?}"),
    }
}

fn limit(window: Window, percent: f64, resets_ms: Option<i64>, severity: Severity) -> Limit {
    Limit {
        window,
        percent,
        resets_ms,
        severity,
    }
}

#[test]
fn claude_codes_answer_lists_its_limits_in_order_with_their_reset_times() {
    assert_eq!(
        limits_of(&claude_answer()),
        [
            limit(
                Window::Session,
                3.0,
                Some(1_792_437_599_661),
                Severity::Normal
            ),
            limit(
                Window::Week,
                86.0,
                Some(1_792_677_599_661),
                Severity::Warning
            ),
            limit(
                Window::WeekOf("Fable".into()),
                0.0,
                Some(1_792_677_600_000),
                Severity::Normal
            ),
        ]
    );
}

#[test]
fn a_claude_limit_scoped_to_a_surface_is_named_after_it() {
    let line = limits_with(
        r#"[{"kind":"weekly_scoped","percent":5,"severity":"normal","scope":{"model":null,"surface":{"display_name":"Cowork"}}}]"#,
    );
    assert_eq!(limits_of(&line)[0].window, Window::WeekOf("Cowork".into()));
}

#[test]
fn a_claude_limit_famulen_has_no_name_for_keeps_claude_codes() {
    let got = limits_of(&limits_with(
        r#"[{"kind":"monthly_all","percent":12.5,"severity":"normal","resets_at":"soon"}]"#,
    ));
    assert_eq!(
        got,
        [limit(
            Window::Other("monthly_all".into()),
            12.5,
            None,
            Severity::Normal
        )]
    );
    assert_eq!(got[0].window.label(), "monthly all");
}

#[test]
fn a_claude_severity_past_warning_is_critical() {
    let got = limits_of(&limits_with(
        r#"[{"kind":"session","percent":100,"severity":"exceeded"}]"#,
    ));
    assert_eq!(got[0].severity, Severity::Critical);
}

#[test]
fn a_claude_severity_famulen_does_not_know_is_read_from_the_percent() {
    let got = limits_of(&limits_with(
        r#"[{"kind":"session","percent":5,"severity":"info"},{"kind":"weekly_all","percent":95}]"#,
    ));
    assert_eq!(got[0].severity, Severity::Normal);
    assert_eq!(got[1].severity, Severity::Critical);
}

#[test]
fn a_claude_limit_without_a_kind_or_a_percent_is_passed_over() {
    let got = limits_of(&limits_with(
        r#"[{"percent":3},{"kind":"session"},{"kind":"weekly_all","percent":1}]"#,
    ));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].window, Window::Week);
}

#[test]
fn a_claude_answer_without_limits_is_no_answer() {
    let line = control_success(r#"{"rate_limits_available":false,"rate_limits":null}"#);
    assert_eq!(claude_reply(&line), Reply::Refused);
}

#[test]
fn a_claude_error_is_a_refusal_and_other_lines_are_passed_over() {
    let error = r#"{"type":"control_response","response":{"subtype":"error","request_id":"famulen","error":"Unknown subtype"}}"#;
    assert_eq!(claude_reply(error), Reply::Refused);
    let others = r#"{"type":"control_response","response":{"subtype":"success","request_id":"x","response":{}}}"#;
    assert_eq!(claude_reply(others), Reply::Other);
    assert_eq!(
        claude_reply(r#"{"type":"system","subtype":"init"}"#),
        Reply::Other
    );
    assert_eq!(claude_reply("not json"), Reply::Other);
}

#[test]
fn the_claude_request_asks_only_for_usage_without_the_accounts_habits() {
    let sent: Value = serde_json::from_str(CLAUDE_REQUEST.trim_end()).unwrap();
    assert_eq!(sent["type"], "control_request");
    assert_eq!(sent["request"]["subtype"], "get_usage");
    assert_eq!(sent["request"]["skip_behaviors"], true);
    assert_eq!(sent["request_id"], REQUEST_ID);
    assert!(CLAUDE_REQUEST.ends_with('\n'));
}

#[test]
fn claude_code_is_started_without_a_session_or_the_owners_settings() {
    assert!(CLAUDE_ARGS.contains(&"--no-session-persistence"));
    assert!(CLAUDE_ARGS.contains(&"--strict-mcp-config"));
    let at = CLAUDE_ARGS
        .iter()
        .position(|a| *a == "--setting-sources")
        .unwrap();
    assert_eq!(CLAUDE_ARGS[at + 1], "");
}

#[test]
fn codex_windows_keep_their_duration_usage_and_reset_time() {
    let got = codex_limits(&json!({"rateLimits": {
        "primary": {"usedPercent": 23, "windowDurationMins": 300, "resetsAt": 1792500000_i64},
        "secondary": {"usedPercent": 92, "windowDurationMins": 10080, "resetsAt": 1792600000_i64}
    }}))
    .unwrap();
    assert_eq!(
        got,
        [
            limit(
                Window::Other("5 時間".into()),
                23.0,
                Some(1_792_500_000_000),
                Severity::Normal
            ),
            limit(
                Window::Week,
                92.0,
                Some(1_792_600_000_000),
                Severity::Critical
            ),
        ]
    );
}

#[test]
fn a_single_codex_bucket_is_not_named_and_its_legacy_copy_is_not_doubled() {
    let bucket = json!({"limitId": "codex", "limitName": null,
        "primary": {"usedPercent": 1, "windowDurationMins": 10080, "resetsAt": 1796194416_i64},
        "secondary": null});
    let got =
        codex_limits(&json!({"rateLimits": bucket, "rateLimitsByLimitId": {"codex": bucket}}))
            .unwrap();
    assert_eq!(
        got,
        [limit(
            Window::Week,
            1.0,
            Some(1_796_194_416_000),
            Severity::Normal
        )]
    );
}

#[test]
fn codex_buckets_beside_the_accounts_own_are_named_after_their_limits() {
    let bucket = |name: &str| {
        json!({"limitName": name,
            "primary": {"usedPercent": 85, "windowDurationMins": 90, "resetsAt": null}})
    };
    let got = codex_limits(&json!({"rateLimits": bucket("Codex"),
        "rateLimitsByLimitId": {"codex": bucket("Codex"), "other": bucket("Other model")}
    }))
    .unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].window, Window::Other("90 分".into()));
    assert_eq!(got[1].window, Window::Other("90 分・Other model".into()));
    assert_eq!(got[0].severity, Severity::Warning);
    assert_eq!(got[0].resets_ms, None);
}

#[test]
fn missing_codex_limits_are_unknown_and_empty_ones_are_empty() {
    assert_eq!(codex_limits(&json!({})), None);
    assert_eq!(
        codex_limits(&json!({"rateLimits": {"primary": null, "secondary": null}})),
        Some(vec![])
    );
}

#[test]
fn windows_are_named_for_the_owner() {
    assert_eq!(Window::Session.label(), "5 時間");
    assert_eq!(Window::Week.label(), "週");
    assert_eq!(Window::WeekOf("Fable".into()).label(), "週・Fable");
}

fn spec(kind: ModelKind, cli: Cli, endpoint: Option<&str>) -> ModelSpec {
    ModelSpec {
        id: "m".into(),
        name: "m".into(),
        kind,
        endpoint: endpoint.map(str::to_owned),
        model: String::new(),
        cli,
        price: Price::default(),
    }
}

#[test]
fn only_an_agent_cli_has_an_account_known_by_its_program() {
    let account = |cli, executable: &str| {
        Some(Account {
            cli,
            executable: executable.into(),
        })
    };
    let of = |kind, cli, endpoint| Account::of(&spec(kind, cli, endpoint), "/opt/claude");
    assert_eq!(
        of(ModelKind::AgentCli, Cli::Claude, None),
        account(Cli::Claude, "/opt/claude")
    );
    assert_eq!(
        of(ModelKind::AgentCli, Cli::Claude, Some("/my/claude")),
        account(Cli::Claude, "/my/claude")
    );
    assert_eq!(
        of(ModelKind::AgentCli, Cli::Codex, None),
        account(Cli::Codex, "codex")
    );
    assert_eq!(
        of(ModelKind::Api, Cli::Claude, Some("https://api.example.com")),
        None
    );
}

fn account() -> Account {
    Account {
        cli: Cli::Claude,
        executable: "claude".into(),
    }
}

#[tokio::test]
async fn limits_are_asked_for_once_while_they_are_fresh() {
    let limits = Limits::default();
    let asked = AtomicUsize::new(0);
    let ask = async |_: &Account| {
        asked.fetch_add(1, Ordering::SeqCst);
        Some(vec![limit(Window::Session, 3.0, None, Severity::Normal)])
    };
    let t = Instant::now();
    let first = limits.read(&account(), t, ask).await;
    let again = limits
        .read(&account(), t + LIMITS_TTL - Duration::from_secs(1), ask)
        .await;
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(first, again);
    limits.read(&account(), t + LIMITS_TTL, ask).await;
    assert_eq!(asked.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_failure_keeps_the_limits_there_were_and_waits_before_asking_again() {
    let limits = Limits::default();
    let t = Instant::now();
    let held = vec![limit(Window::Week, 30.0, None, Severity::Normal)];
    limits
        .read(&account(), t, async |_: &Account| Some(held.clone()))
        .await;
    let failed = limits
        .read(&account(), t + LIMITS_TTL, async |_: &Account| None)
        .await;
    assert_eq!(failed, Some(held.clone()));
    let asked = AtomicUsize::new(0);
    limits
        .read(
            &account(),
            t + LIMITS_TTL * 2 - Duration::from_secs(1),
            async |_: &Account| {
                asked.fetch_add(1, Ordering::SeqCst);
                None
            },
        )
        .await;
    assert_eq!(asked.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_account_that_never_answered_has_no_limits() {
    let limits = Limits::default();
    let got = limits
        .read(&account(), Instant::now(), async |_: &Account| None)
        .await;
    assert_eq!(got, None);
}

/// `sh` running `script` in a directory of its own, standing in for a CLI;
/// its arguments follow.
#[cfg(unix)]
fn fake(script: &str) -> (std::path::PathBuf, Command) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "famulen-limits-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cli.sh");
    std::fs::write(&path, script).unwrap();
    let mut command = Command::new("sh");
    command.arg(&path);
    (dir, command)
}

#[cfg(unix)]
#[tokio::test]
async fn the_claude_probe_writes_the_usage_request_and_reads_the_answer() {
    let (dir, command) = fake(&format!(
        "IFS= read -r line\nprintf '%s\\n' \"$line\" > \"$(dirname \"$0\")/stdin\"\nprintf '%s\\n' \"$*\" > \"$(dirname \"$0\")/args\"\necho '{{\"type\":\"system\"}}'\necho '{}'\nexec sleep 30",
        claude_answer()
    ));
    let got = probe_with(Cli::Claude, command, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(got.len(), 3);
    let written = std::fs::read_to_string(dir.join("stdin")).unwrap();
    assert_eq!(written, CLAUDE_REQUEST);
    let args = std::fs::read_to_string(dir.join("args")).unwrap();
    assert!(args.contains("--input-format stream-json"), "{args}");
}

#[cfg(unix)]
#[tokio::test]
async fn the_codex_probe_initializes_then_reads_limits_without_starting_a_turn() {
    let (dir, command) = fake(
        r#"
test "$1" = app-server || exit 1
IFS= read -r line
printf '%s\n' "$line" > "$(dirname "$0")/requests"
echo '{"id":"famulen-init","result":{}}'
IFS= read -r line
printf '%s\n' "$line" >> "$(dirname "$0")/requests"
IFS= read -r line
printf '%s\n' "$line" >> "$(dirname "$0")/requests"
echo '{"method":"account/updated","params":{}}'
echo '{"id":"unrelated","result":{}}'
echo '{"id":"famulen-query","result":{"rateLimits":{"primary":{"usedPercent":25,"windowDurationMins":300}}}}'
exec sleep 30
"#,
    );
    let got = probe_with(Cli::Codex, command, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].percent, 25.0);
    let requests = std::fs::read_to_string(dir.join("requests")).unwrap();
    let methods: Vec<String> = requests
        .lines()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["method"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(
        methods,
        ["initialize", "initialized", "account/rateLimits/read"]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_refusal_an_ended_output_or_silence_leaves_the_limits_unknown() {
    let refusal = r#"{"type":"control_response","response":{"subtype":"error","request_id":"famulen","error":"Unknown subtype"}}"#;
    for (cli, script) in [
        (
            Cli::Claude,
            format!("read line\necho '{refusal}'\nexec sleep 30"),
        ),
        (Cli::Claude, "exit 0".to_owned()),
        (Cli::Claude, "exec sleep 30".to_owned()),
        (
            Cli::Codex,
            "read line\necho '{\"id\":\"famulen-init\",\"error\":{\"code\":-1}}'\nexec sleep 30"
                .to_owned(),
        ),
    ] {
        let (_dir, command) = fake(&script);
        let got = probe_with(cli, command, Duration::from_millis(500)).await;
        assert_eq!(got, None, "{script}");
    }
}

#[tokio::test]
async fn a_program_that_is_not_there_leaves_the_limits_unknown() {
    let account = Account {
        cli: Cli::Codex,
        executable: "/nonexistent/famulen-codex".into(),
    };
    assert_eq!(probe(&account).await, None);
}

/// Needs a `claude` on PATH logged in to a plan with limits, and the
/// network, which CI has not.
#[tokio::test]
#[ignore = "needs Claude Code installed and logged in to a plan with limits"]
async fn the_installed_claude_code_answers_with_its_limits() {
    let got = probe(&account()).await.expect("an answer");
    assert!(!got.is_empty(), "{got:?}");
}

#[tokio::test]
#[ignore = "needs Codex installed and logged in to a plan with limits"]
async fn the_installed_codex_answers_with_its_limits() {
    let got = probe(&Account {
        cli: Cli::Codex,
        executable: "codex".into(),
    })
    .await
    .expect("an answer");
    assert!(!got.is_empty(), "{got:?}");
}
