use super::*;

fn content() -> Content {
    Content {
        changes_ci: false,
        change: "c".into(),
        commit: "a".repeat(40),
        tree: "t".into(),
        base: "b".repeat(40),
        title: "Add b".into(),
        body: String::new(),
    }
}

#[test]
fn a_review_ends_with_its_verdict_and_one_without_is_none() {
    assert_eq!(verdict("Fine.\nVERDICT: clean\n").unwrap(), Verdict::Clean);
    assert_eq!(
        verdict("a.rs:1 is off\n\n`VERDICT: findings`").unwrap(),
        Verdict::Findings {
            text: "a.rs:1 is off".into(),
            serious: false
        }
    );
    assert_eq!(
        verdict("leaks\nVERDICT: serious").unwrap(),
        Verdict::Findings {
            text: "leaks".into(),
            serious: true
        }
    );
    assert!(verdict("VERDICT: clean\nbut one more thing").is_err());
    assert!(verdict("").is_err());
}

#[test]
fn changes_too_large_to_be_read_whole_are_not_asked_of_a_review() {
    let asked = review_message("do b", &content(), "+b").unwrap();
    assert!(asked.contains("do b") && asked.ends_with("+b"), "{asked}");
    assert_eq!(
        review_message("do b", &content(), &"+".repeat(DIFF_CHARS + 1)),
        None
    );
}

#[test]
fn a_review_is_handed_nothing_that_writes() {
    for tool in REVIEW_TOOLS {
        assert!(["Read", "Grep", "Glob"].contains(tool), "{tool}");
    }
}
