//! Which CJK scripts a collection uses, so the app can fetch the fonts that draw them.

/// Whether the deck files contain Hangul, kana or Han characters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct Scripts {
    pub hangul: bool,
    pub kana: bool,
    pub han: bool,
}

impl Scripts {
    /// Adds the scripts used in `text`.
    pub fn scan(&mut self, text: &str) {
        for c in text.chars() {
            if self.hangul && self.kana && self.han {
                return;
            }
            // Below U+1100 every character is Latin, Greek, Cyrillic or the like.
            if c < '\u{1100}' {
                continue;
            }
            match c {
                '\u{1100}'..='\u{11FF}'
                | '\u{3130}'..='\u{318F}'
                | '\u{A960}'..='\u{A97F}'
                | '\u{AC00}'..='\u{D7FF}'
                | '\u{FFA0}'..='\u{FFDC}' => self.hangul = true,
                '\u{3040}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9F}' => self.kana = true,
                '\u{3400}'..='\u{4DBF}'
                | '\u{4E00}'..='\u{9FFF}'
                | '\u{F900}'..='\u{FAFF}'
                | '\u{20000}'..='\u{3134F}' => self.han = true,
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(text: &str) -> Scripts {
        let mut scripts = Scripts::default();
        scripts.scan(text);
        scripts
    }

    #[test]
    fn finds_each_script() {
        assert_eq!(scan("Q: capital of France?\nA: Paris — café"), Scripts::default());
        assert_eq!(scan("안녕하세요"), Scripts { hangul: true, ..Default::default() });
        assert_eq!(scan("ひらがな カタカナ"), Scripts { kana: true, ..Default::default() });
        assert_eq!(scan("水"), Scripts { han: true, ..Default::default() });
        assert_eq!(scan("日本語のテキスト"), Scripts { kana: true, han: true, ..Default::default() });
    }

    #[test]
    fn adds_to_what_was_found() {
        let mut scripts = scan("한국어");
        scripts.scan("漢字");
        assert_eq!(scripts, Scripts { hangul: true, han: true, kana: false });
    }
}
