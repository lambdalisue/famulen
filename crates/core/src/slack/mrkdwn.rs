//! Markdown as Slack's mrkdwn. What is already mrkdwn stays as it is, so
//! either reads right.

/// `text`, written in Markdown or mrkdwn, as mrkdwn.
pub fn to_mrkdwn(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("```") {
            let opening = line.trim().trim_start_matches('`');
            if opening.contains("```") {
                out.push(inline(line));
                i += 1;
                continue;
            }
            // Slack shows a language tag as part of the code; mrkdwn may
            // start the code itself there.
            let code = if is_info(opening) {
                String::new()
            } else {
                escape(opening)
            };
            out.push(format!("```{code}"));
            i += 1;
            loop {
                let Some(line) = lines.get(i) else {
                    out.push("```".into());
                    break;
                };
                i += 1;
                if line.trim_start().starts_with("```") {
                    out.push("```".into());
                    break;
                }
                out.push(escape(line));
                // mrkdwn may close it at the end of a line; Markdown has the
                // same there as code, and a fence of its own to close it.
                if line.trim_end().ends_with("```") && closed_by_fences(&lines[i..]) {
                    break;
                }
            }
        } else if is_row(line) && lines.get(i + 1).is_some_and(|l| is_rule_row(l)) {
            out.push("```".into());
            while i < lines.len() && is_row(lines[i]) {
                out.push(escape(lines[i]));
                i += 1;
            }
            out.push("```".into());
        } else {
            out.push(block(line));
            i += 1;
        }
    }
    out.join("\n")
}

/// Whether what follows an opening fence says what the code is, as Markdown
/// has it: a language, and perhaps attributes.
fn is_info(opening: &str) -> bool {
    let mut words = opening.split_whitespace();
    words.next();
    words
        .next()
        .is_none_or(|w| w.starts_with('{') || (w.contains('=') && !w.starts_with('=')))
}

/// Whether the fences of `lines` open and close each other, so none is left
/// to close code from before.
fn closed_by_fences(lines: &[&str]) -> bool {
    let fences = lines.iter().filter(|line| {
        line.trim()
            .strip_prefix("```")
            .is_some_and(|rest| !rest.contains("```"))
    });
    fences.count().is_multiple_of(2)
}

fn is_row(line: &str) -> bool {
    line.trim_start().starts_with('|')
}

fn is_rule_row(line: &str) -> bool {
    is_row(line) && line.contains('-') && line.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// One line outside code.
fn block(line: &str) -> String {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let hashes = body.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
        let title = inline(body[hashes..].trim()).replace('*', "");
        return format!("*{title}*");
    }
    if body.len() >= 3 && (body.chars().all(|c| c == '-') || body.chars().all(|c| c == '*')) {
        return "———".into();
    }
    if let Some(quoted) = body.strip_prefix('>') {
        return format!("{indent}>{}", inline(quoted));
    }
    for (mark, shown) in [
        ("- [ ] ", "☐ "),
        ("- [x] ", "☑ "),
        ("- [X] ", "☑ "),
        ("- ", "• "),
        ("* ", "• "),
        ("+ ", "• "),
    ] {
        if let Some(item) = body.strip_prefix(mark) {
            return format!("{indent}{shown}{}", inline(item));
        }
    }
    format!("{indent}{}", inline(body))
}

/// Emphasis, links and code within a line.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let before = out.chars().last();
        if let Some((code, after)) = code(rest) {
            out.push_str(&escape(code));
            rest = after;
        } else if let Some((url, after)) = bare_url(rest) {
            out.push_str(&escape(url));
            rest = after;
        } else if let Some((control, after)) = control(rest) {
            out.push_str(control);
            rest = after;
        } else if let Some((label, url, after)) = link(rest.strip_prefix('!').unwrap_or(rest)) {
            let label = label.replace(['|', '<', '>'], " ");
            out.push_str(&format!("<{url}|{}>", label.trim()));
            rest = after;
        } else if let Some((mark, inner, after)) = emphasis(rest, before) {
            emphasize(&mut out, mark, &inline(inner), after);
            rest = after;
        } else if let Some(entity) = entity(rest) {
            out.push_str(entity);
            rest = &rest[entity.len()..];
        } else {
            out.push_str(&escape(&rest[..c.len_utf8()]));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The code `text` starts with, with its marks, and what follows.
fn code(text: &str) -> Option<(&str, &str)> {
    let marks = &text[..text.len() - text.trim_start_matches('`').len()];
    if marks.is_empty() {
        return None;
    }
    let (inner, _) = between(text, marks, marks)?;
    Some(text.split_at(marks.len() * 2 + inner.len()))
}

/// The address `text` starts with, up to where an address or a mark ends.
fn bare_url(text: &str) -> Option<(&str, &str)> {
    if !["http://", "https://"].iter().any(|s| text.starts_with(s)) {
        return None;
    }
    // Letters beyond ASCII may be of the address or of the sentence around
    // it: emphasis after them is the sentence's, an underscore the address's.
    let mut beyond = false;
    let end = text
        .find(|c: char| {
            beyond |= !c.is_ascii();
            let part = if c.is_ascii() {
                c.is_ascii_graphic() && !"<>`|)]\"".contains(c)
            } else {
                c.is_alphanumeric()
            };
            !part || (beyond && matches!(c, '*' | '~'))
        })
        .unwrap_or(text.len());
    // A mark that closes right after the address is no part of it.
    let url = text[..end].trim_end_matches(['*', '_', '~']);
    Some(text.split_at(url.len()))
}

/// The emphasis `text`, standing after `before`, starts with: its mrkdwn
/// mark, what is inside, and what follows.
fn emphasis(text: &str, before: Option<char>) -> Option<(&'static str, &str, &str)> {
    let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');

    for (open, mark) in [
        ("***", "*_"),
        ("**", "*"),
        ("__", "*"),
        ("~~", "~"),
        ("*", "*"),
        ("_", "_"),
    ] {
        if let Some((inner, after)) = between(text, open, open)
            && !inner.starts_with(char::is_whitespace)
            && !inner.ends_with(char::is_whitespace)
            && !(open == "__" && after.starts_with(|c: char| c.is_alphanumeric()))
            && !(open == "_" && (word(before) || word(after.chars().next())))
            && !(open == "*"
                && word(before)
                && word(inner.chars().next())
                && word(inner.chars().last()))
        {
            return Some((mark, inner, after));
        }
    }
    None
}

/// Put `inner` in `mark`, apart from what stands beside it: Slack reads a
/// mark only at the edge of a word, and Japanese has none.
fn emphasize(out: &mut String, mark: &str, inner: &str, after: &str) {
    let apart = |c: char| c.is_whitespace() || c.is_ascii_punctuation();
    if out.chars().last().is_some_and(|c| !apart(c)) {
        out.push(' ');
    }
    let close: String = mark.chars().rev().collect();
    out.push_str(mark);
    out.push_str(inner);
    out.push_str(&close);
    if after.chars().next().is_some_and(|c| !apart(c)) {
        out.push(' ');
    }
}

/// What stands between `open` at the start of `text` and the next `close`.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<(&'a str, &'a str)> {
    let body = text.strip_prefix(open)?;
    let end = body.find(close).filter(|end| *end > 0)?;
    Some((&body[..end], &body[end + close.len()..]))
}

/// A mention, a channel or a link already as Slack writes them.
fn control(text: &str) -> Option<(&str, &str)> {
    let known = ["<@", "<#", "<!", "<http://", "<https://", "<mailto:"];
    if !known.iter().any(|k| text.starts_with(k)) {
        return None;
    }
    let end = text.find('>')? + 1;
    Some((&text[..end], &text[end..]))
}

fn link(text: &str) -> Option<(&str, &str, &str)> {
    let (label, after) = between(text, "[", "]")?;
    let (url, after) = between(after, "(", ")")?;
    let usable = ["http://", "https://", "mailto:"]
        .iter()
        .any(|s| url.starts_with(s));
    (usable && !url.contains(char::is_whitespace)).then_some((label, url, after))
}

/// One of the three characters already escaped.
fn entity(text: &str) -> Option<&'static str> {
    ["&amp;", "&lt;", "&gt;"]
        .into_iter()
        .find(|e| text.starts_with(e))
}

/// The three characters Slack reads as its own.
fn escape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some(entity) = entity(rest) {
            out.push_str(entity);
            rest = &rest[entity.len()..];
            continue;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_mrkdwn;

    #[test]
    fn markdown_becomes_mrkdwn() {
        for (markdown, mrkdwn) in [
            ("plain, 100 items", "plain, 100 items"),
            ("a **bold** b", "a *bold* b"),
            ("***both***", "*_both_*"),
            ("~~gone~~", "~gone~"),
            ("## 今日の **予定**", "*今日の 予定*"),
            (
                "[案内](https://a.example/x?y=1)",
                "<https://a.example/x?y=1|案内>",
            ),
            (
                "![図](https://a.example/p.png)",
                "<https://a.example/p.png|図>",
            ),
            ("- a\n  - b\n* c", "• a\n  • b\n• c"),
            ("- [ ] まだ\n- [x] 済み", "☐ まだ\n☑ 済み"),
            ("1. a\n2. b", "1. a\n2. b"),
            ("> 引用 **太字**", "> 引用 *太字*"),
            ("a\n\n---\n\nb", "a\n\n———\n\nb"),
            ("5 < 6 & 7 > 2", "5 &lt; 6 &amp; 7 &gt; 2"),
        ] {
            assert_eq!(to_mrkdwn(markdown), mrkdwn, "{markdown}");
        }
    }

    #[test]
    fn bold_is_set_apart_from_japanese_beside_it() {
        for (markdown, mrkdwn) in [
            ("これは**大事**です", "これは *大事* です"),
            ("**「重要」**、です", "*「重要」* 、です"),
            ("(**a**)", "(*a*)"),
            ("これは *大事* です", "これは *大事* です"),
            ("これは*大事*です", "これは *大事* です"),
            ("これは_大事_です", "これは _大事_ です"),
            ("_大事_です", "_大事_ です"),
            ("これは_大事_", "これは _大事_"),
            ("これは _大事_ です", "これは _大事_ です"),
            ("_a_、_b_。", "_a_ 、 _b_ 。"),
            ("（*a*）と（_b_）", "（ *a* ）と（ _b_ ）"),
            ("**_両方_**です", "*_両方_* です"),
            (
                "https://a.example/xを**必ず**見る",
                "https://a.example/xを *必ず* 見る",
            ),
            (
                "https://a.example/x、_必ず_",
                "https://a.example/x、 _必ず_",
            ),
            ("APIは**大事**", "APIは *大事*"),
        ] {
            assert_eq!(to_mrkdwn(markdown), mrkdwn, "{markdown}");
        }
    }

    #[test]
    fn code_is_left_as_written() {
        for (markdown, mrkdwn) in [
            ("`**a** <b>` と**c**", "`**a** &lt;b&gt;` と *c*"),
            (
                "```rust\nlet a = b && **c;\n```\n**d**",
                "```\nlet a = b &amp;&amp; **c;\n```\n*d*",
            ),
            (
                "```js\n// ```\nconst a = '<@U1>';\n```\n**d**",
                "```\n// ```\nconst a = '&lt;@U1&gt;';\n```\n*d*",
            ),
            (
                "```\na```\n**b**\n```\n**c**\n```",
                "```\na```\n*b*\n```\n**c**\n```",
            ),
            ("```python title=\"my example.py\"\na\n```", "```\na\n```"),
            ("```js {1,3}\na\n```", "```\na\n```"),
            (
                "| a | b |\n|---|---|\n| **1** | 2 |\n後",
                "```\n| a | b |\n|---|---|\n| **1** | 2 |\n```\n後",
            ),
        ] {
            assert_eq!(to_mrkdwn(markdown), mrkdwn, "{markdown}");
        }
    }

    #[test]
    fn what_slack_writes_itself_and_what_is_no_mark_stay() {
        for same in [
            "<@U123> と <#C1|general> と <https://a.example|案内>",
            "snake_case_name と __init__x",
            "2 * 3 * 4",
            "a ** b",
            "&lt;tag&gt;",
            "・項目",
            "*太字* と _斜体_ と ~取り消し~",
            "*_両方_*",
            "項目 snake_case_の名前",
            "2*3*4",
            "https://a.example/__init__/*x*?a=_b_",
            "https://a.example/日本語_資料_最新版",
            "```\n**a**\nb```\nc",
            "```let a = 1;\nb\n```",
            "前 ```**a** _b_``` 後",
            "``**a** ` b``",
        ] {
            assert_eq!(to_mrkdwn(same), same);
        }
        assert_eq!(
            to_mrkdwn(&to_mrkdwn("これは**大事**です")),
            "これは *大事* です"
        );
    }
}
