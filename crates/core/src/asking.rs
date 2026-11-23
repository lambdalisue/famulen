//! Questions put to the owner on Slack, answered by a button or in their own
//! words.

use serde_json::{Value, json};

use famulen_gateway::Question;

/// The action ids of a question's own controls begin with this.
pub const PREFIX: &str = "famulen.question.";
const PICK: &str = "famulen.question.pick.";
const SEND: &str = "famulen.question.send";
const TEXT: &str = "famulen.question.text";
const CHOOSE: &str = "famulen.question.choose";

/// What Slack takes: choices in one field, a label's length, a section's.
const MOST_CHOICES: usize = 10;
const LABEL_CHARS: usize = 75;
const SECTION_CHARS: usize = 2900;

/// Whether one press of a choice answers it all.
fn quick(questions: &[Question]) -> bool {
    matches!(questions, [q] if !q.several && !q.options.is_empty())
}

fn plain(text: &str, most: usize) -> Value {
    let mut cut: String = text.chars().take(most).collect();
    if text.chars().count() > most {
        cut.pop();
        cut.push('…');
    }
    json!({ "type": "plain_text", "text": cut, "emoji": true })
}

/// The blocks that ask `questions` in request `id`; none when Slack cannot
/// show them whole.
pub fn blocks(id: i64, questions: &[Question]) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        let choices: String = q
            .options
            .iter()
            .map(|c| {
                let about = if c.description.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", c.description)
                };
                format!("\n• {}{about}", c.label)
            })
            .collect();
        let text = format!("{}{choices}", q.question);
        if text.chars().count() > SECTION_CHARS || q.options.len() > MOST_CHOICES {
            return None;
        }
        out.push(json!({ "type": "section", "text": plain(&text, SECTION_CHARS) }));
        if quick(questions) {
            let buttons: Vec<Value> = q
                .options
                .iter()
                .enumerate()
                .map(|(n, c)| {
                    json!({
                        "type": "button",
                        "text": plain(&c.label, LABEL_CHARS),
                        "action_id": format!("{PICK}{n}"),
                        "value": id.to_string(),
                    })
                })
                .collect();
            out.push(json!({ "type": "actions", "elements": buttons }));
        } else if !q.options.is_empty() {
            let options: Vec<Value> = q
                .options
                .iter()
                .enumerate()
                .map(|(n, c)| json!({ "text": plain(&c.label, LABEL_CHARS), "value": n.to_string() }))
                .collect();
            let kind = if q.several {
                "checkboxes"
            } else {
                "radio_buttons"
            };
            out.push(json!({
                "type": "input",
                "block_id": format!("choose-{i}"),
                "optional": true,
                "label": plain("選ぶ", LABEL_CHARS),
                "element": { "type": kind, "action_id": CHOOSE, "options": options },
            }));
        }
        out.push(json!({
            "type": "input",
            "block_id": format!("text-{i}"),
            "optional": true,
            "label": plain("自分の言葉で答える", LABEL_CHARS),
            "element": { "type": "plain_text_input", "action_id": TEXT },
        }));
    }
    out.push(json!({
        "type": "actions",
        "elements": [{
            "type": "button",
            "style": "primary",
            "text": plain(if quick(questions) { "この文で答える" } else { "答える" }, LABEL_CHARS),
            "action_id": SEND,
            "value": id.to_string(),
        }],
    }));
    Some(out)
}

/// The answers a press of one of the controls gives, one for each question;
/// `None` while a question has none.
pub fn replies(action_id: &str, fields: &Value, questions: &[Question]) -> Option<Vec<String>> {
    if let Some(n) = action_id.strip_prefix(PICK) {
        let [q] = questions else {
            return None;
        };
        return Some(vec![q.options.get(n.parse::<usize>().ok()?)?.label.clone()]);
    }
    if action_id != SEND {
        return None;
    }
    questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let written = fields[format!("text-{i}")][TEXT]["value"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            if let Some(written) = written {
                return Some(written.to_owned());
            }
            let chosen = &fields[format!("choose-{i}")][CHOOSE];
            let picked: Vec<&Value> = match chosen["selected_options"].as_array() {
                Some(several) => several.iter().collect(),
                None => vec![&chosen["selected_option"]],
            };
            let labels: Vec<&str> = picked
                .iter()
                .filter_map(|o| o["value"].as_str()?.parse::<usize>().ok())
                .filter_map(|n| Some(q.options.get(n)?.label.as_str()))
                .collect();
            (!labels.is_empty()).then(|| labels.join(", "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use famulen_gateway::Choice;

    fn question(several: bool) -> Question {
        Question {
            question: "好きな色は？".into(),
            options: ["赤", "青"]
                .map(|label| Choice {
                    label: label.into(),
                    description: String::new(),
                })
                .into(),
            several,
        }
    }

    #[test]
    fn one_question_is_answered_by_one_press_or_in_words() {
        let one = [question(false)];
        let shown = Value::Array(blocks(7, &one).unwrap()).to_string();
        assert!(shown.contains("famulen.question.pick.1") && shown.contains("plain_text_input"));

        assert_eq!(
            replies("famulen.question.pick.1", &Value::Null, &one),
            Some(vec!["青".into()])
        );
        assert_eq!(replies("famulen.question.pick.9", &Value::Null, &one), None);
        let words = json!({ "text-0": { TEXT: { "type": "plain_text_input", "value": " 緑 " } } });
        assert_eq!(replies(SEND, &words, &one), Some(vec!["緑".into()]));
        let nothing = json!({ "text-0": { TEXT: { "value": null } } });
        assert_eq!(replies(SEND, &nothing, &one), None);
    }

    #[test]
    fn several_questions_are_answered_together() {
        let two = [question(false), question(true)];
        let shown = Value::Array(blocks(7, &two).unwrap()).to_string();
        assert!(!shown.contains(PICK) && shown.contains("checkboxes"));

        let fields = json!({
            "choose-0": { CHOOSE: { "selected_option": { "value": "0" } } },
            "choose-1": { CHOOSE: { "selected_options": [{ "value": "0" }, { "value": "1" }] } },
        });
        assert_eq!(
            replies(SEND, &fields, &two),
            Some(vec!["赤".into(), "赤, 青".into()])
        );
        let half = json!({ "choose-0": { CHOOSE: { "selected_option": { "value": "0" } } } });
        assert_eq!(replies(SEND, &half, &two), None, "one is not answered");
    }

    #[test]
    fn what_slack_cannot_show_whole_is_not_asked_there() {
        let mut long = question(false);
        long.question = "あ".repeat(SECTION_CHARS);
        assert_eq!(blocks(7, &[long]), None);
    }
}
