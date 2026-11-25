//! What every adapter keeps to, checked on lines recorded of its agent.

use serde_json::json;

use super::*;

/// Lines an agent wrote, by what they are.
pub(super) struct Recorded {
    pub(super) starts: &'static str,
    pub(super) says: &'static str,
    pub(super) calls: &'static str,
    pub(super) answers: &'static str,
    pub(super) asks: &'static str,
    /// A question no adapter knows.
    pub(super) asks_unknown: &'static str,
    pub(super) ends_turn: &'static str,
    /// A kind of line no adapter knows.
    pub(super) unknown: &'static str,
}

pub(super) fn check(adapter: &dyn Adapter, recorded: &Recorded) {
    let mut codec = adapter.codec();
    let mut read = |line: &str| codec.read(line);

    assert!(
        read(recorded.starts)
            .iter()
            .any(|b| matches!(b, Body::Started { conversation } if !conversation.is_empty()))
    );
    assert!(
        read(recorded.says)
            .iter()
            .any(|b| matches!(b, Body::Said { text, .. } if !text.is_empty()))
    );
    assert!(
        read(recorded.calls)
            .iter()
            .any(|b| matches!(b, Body::ToolCalled { tool, .. } if !tool.is_empty()))
    );
    assert!(
        read(recorded.answers)
            .iter()
            .any(|b| matches!(b, Body::ToolAnswered { .. }))
    );
    let asked = question(read(recorded.asks));
    assert!(matches!(asked.ask, Ask::Tool { .. }), "{asked:?}");
    let unknown_bodies = read(recorded.asks_unknown);
    assert!(
        unknown_bodies
            .iter()
            .any(|b| matches!(b, Body::Warned { .. }))
    );
    let unknown = question(unknown_bodies);
    assert!(matches!(unknown.ask, Ask::Unknown { .. }), "{unknown:?}");
    for line in [recorded.unknown, "not a line of its protocol", "{}"] {
        assert!(
            matches!(read(line)[..], [Body::Warned { .. }]),
            "{line} is kept as a warning"
        );
    }
    let ended = read(recorded.ends_turn);
    assert!(
        ended
            .iter()
            .any(|b| matches!(b, Body::TurnEnded { ok: true, .. }))
    );
    assert!(ended.iter().any(|b| matches!(
        b,
        Body::Stands {
            standing: Standing::Idle
        }
    )));

    for answer in [Answer::Allow, Answer::Refuse { why: "no".into() }] {
        one_line(adapter.answer(&asked, &answer).unwrap());
    }
    assert!(
        adapter
            .answer(&asked, &Answer::Served { response: None })
            .is_err(),
        "an answer to something else is none"
    );
    // Whatever is said to what is not known, it is refused.
    let refused = adapter.answer(&unknown, &Answer::Allow).unwrap();
    assert_eq!(
        refused,
        adapter.answer(&unknown, &Answer::Unsupported).unwrap()
    );
    one_line(codec.send("a\nb", When::Between).unwrap());
    one_line(codec.interrupt().unwrap());

    let lacking = Work {
        needs: Capabilities {
            sandbox: Sandbox::Whole,
            ..Capabilities::default()
        },
        ..Work::default()
    };
    if adapter.capabilities().sandbox < Sandbox::Whole {
        assert!(adapter.launch(&lacking, None).is_err());
    }
    for line in adapter.launch(&Work::default(), None).unwrap().input {
        one_line(line);
    }
    // An event is the same once it was kept.
    for body in codec
        .read(recorded.asks)
        .into_iter()
        .chain(codec.read(recorded.says))
    {
        let kept = json!(body);
        assert_eq!(serde_json::from_value::<Body>(kept).unwrap(), body);
    }
}

fn question(bodies: Vec<Body>) -> Question {
    bodies
        .into_iter()
        .find_map(|b| match b {
            Body::Asked { question } => Some(question),
            _ => None,
        })
        .expect("a question")
}

fn one_line(line: String) {
    assert!(!line.is_empty() && !line.contains('\n'), "{line:?}");
}
