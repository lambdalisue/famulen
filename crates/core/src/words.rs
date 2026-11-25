//! Which of the names kept a text may speak of, told by the words both hold
//! and without a model.

use std::collections::BTreeSet;

/// How a run of letters is told from the next: Japanese parts no words.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Script {
    Letters,
    Hiragana,
    Katakana,
    Kanji,
}

impl Script {
    fn of(c: char) -> Option<Self> {
        match c {
            'ぁ'..='ゖ' => Some(Self::Hiragana),
            'ァ'..='ヺ' | 'ー' | 'ｦ'..='ﾟ' => Some(Self::Katakana),
            '々' | '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' => Some(Self::Kanji),
            c => c.is_alphanumeric().then_some(Self::Letters),
        }
    }

    /// Shorter runs stand in any text.
    fn shortest(self) -> usize {
        match self {
            Self::Kanji => 2,
            Self::Letters | Self::Katakana => 3,
            Self::Hiragana => 4,
        }
    }
}

/// A number this short is a year, a count or a price in most texts.
const SHORTEST_NUMBER: usize = 5;

/// What stands in the address of nearly every shop, and so in every mail.
const OF_ADDRESSES: [&str; 8] = ["com", "net", "org", "www", "http", "https", "shop", "store"];
/// Full-width katakana, as their half-width forms are ordered from `ｦ` on.
const KATAKANA: &str = "ヲァィゥェォャュョッーアイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワン";

fn written_one_way(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        let after = |by: u32| {
            let last = out.chars().last()?;
            Some((last, char::from_u32(last as u32 + by)?))
        };
        match c {
            '！'..='～' => {
                let c = char::from_u32(c as u32 - 0xfee0).unwrap_or(c);
                out.extend(c.to_lowercase());
            }
            'ｦ'..='ﾝ' => out.extend(KATAKANA.chars().nth(c as usize - 'ｦ' as usize)),
            'ﾞ' | 'ﾟ' => {
                let voiced = match after(if c == 'ﾞ' { 1 } else { 2 }) {
                    Some(('ウ', _)) => Some('ヴ'),
                    Some(('カ'..='ト' | 'ハ'..='ホ', voiced)) => Some(voiced),
                    _ => None,
                };
                if let Some(voiced) = voiced {
                    out.pop();
                    out.push(voiced);
                }
            }
            c => out.extend(c.to_lowercase()),
        }
    }
    out
}

fn runs(text: &str) -> BTreeSet<String> {
    let mut runs = BTreeSet::new();
    let mut run = String::new();
    let mut of = None;
    for c in text.chars().chain([' ']) {
        let script = Script::of(c);
        if script != of {
            if let Some(of) = of
                && !run.is_empty()
            {
                // A name may be written together with its number.
                let before: String = run.chars().take_while(|c| !c.is_numeric()).collect();
                for word in [before, std::mem::take(&mut run)] {
                    if tells(of, &word) {
                        runs.insert(word);
                    }
                }
            }
            run.clear();
            of = script;
        }
        run.push(c);
    }
    runs
}

fn tells(script: Script, run: &str) -> bool {
    let number = run.chars().all(|c| c.is_ascii_digit());
    !OF_ADDRESSES.contains(&run)
        && run.chars().count()
            >= if number {
                SHORTEST_NUMBER
            } else {
                script.shortest()
            }
}

/// A text that names are looked for in.
pub(crate) struct Said {
    text: String,
    words: BTreeSet<String>,
}

impl Said {
    pub(crate) fn new(text: &str) -> Self {
        let text = written_one_way(text);
        let words = runs(&text);
        Self { text, words }
    }

    /// How many words of a name it says. A Japanese word may stand in a longer
    /// one; one of letters is a word by itself. A name of short words alone is
    /// said as a whole or not at all.
    pub(crate) fn words_of(&self, name: &str) -> usize {
        let name = written_one_way(name);
        let runs = runs(&name);
        if runs.is_empty() {
            let name = name.trim();
            return usize::from(!name.is_empty() && self.says_whole(name));
        }
        runs.iter()
            .filter(|word| {
                let letters = word.chars().all(|c| Script::of(c) == Some(Script::Letters));
                self.words.contains(*word) || !letters && self.text.contains(word.as_str())
            })
            .count()
    }

    fn says_whole(&self, name: &str) -> bool {
        let letter = |c: Option<char>| c.and_then(Script::of) == Some(Script::Letters);
        let (first, last) = (name.chars().next(), name.chars().last());
        self.text.match_indices(name).any(|(at, _)| {
            let before = self.text[..at].chars().last();
            let after = self.text[at + name.len()..].chars().next();
            !(letter(first) && letter(before) || letter(last) && letter(after))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_said_by_the_words_it_shares_in_any_script_and_width() {
        let said = Said::new("ＱＮＡＰ製のモバイルバッテリーと暗号の入門書、たまごっちについて");
        assert_eq!(said.words_of("QNAP TS-251D"), 1);
        assert_eq!(said.words_of("大容量バッテリー"), 1);
        assert_eq!(said.words_of("暗号 入門書"), 2);
        assert_eq!(said.words_of("たまごっち"), 1);
        assert_eq!(said.words_of("Raspberry Pi"), 0);
        let said = Said::new("ﾓﾊﾞｲﾙﾊﾞｯﾃﾘｰ と iPhone15 と ｳﾞｨｵﾗ と ﾌﾟﾘﾝﾀ");
        assert_eq!(said.words_of("モバイルバッテリー"), 1);
        assert_eq!(said.words_of("iPhone 15"), 1);
        assert_eq!(said.words_of("ヴィオラ プリンタ"), 2);
    }

    #[test]
    fn a_name_of_short_words_alone_is_said_as_a_whole() {
        let said = Said::new("こたつの自主回収");
        assert_eq!(said.words_of("こたつ"), 1);
        assert_eq!(said.words_of("机"), 0);
        let said = Said::new("audio と au の請求");
        assert_eq!(said.words_of("au"), 1);
        assert_eq!(Said::new("audio php").words_of("au HP"), 0);
        assert_eq!(Said::new("audio php").words_of("HP"), 0);
    }

    #[test]
    fn what_addresses_are_made_of_names_no_shop() {
        let said = Said::new("from: noreply@x.com https://shop.example.net");
        assert_eq!(said.words_of("ヨドバシ.com"), 0);
    }

    #[test]
    fn letters_are_matched_by_whole_words_and_what_any_text_holds_by_none() {
        let said = Said::new("process 2026 年の 3 件について");
        assert_eq!(said.words_of("Pro"), 0);
        assert_eq!(said.words_of("2026 年 の 3"), 0);
        assert_eq!(said.words_of(""), 0);
    }
}
