//! Turning Anki note content into amgi Markdown: template rendering, HTML to
//! inline Markdown, and `{{c1::cloze}}` to `[cloze]`.

use std::collections::{BTreeSet, HashMap};

/// Render an Anki card template. Supports `{{Field}}`, `{{filter:Field}}`,
/// `{{#Field}}…{{/Field}}` and `{{^Field}}…{{/Field}}`. Unknown fields render empty.
pub fn render_template(template: &str, fields: &HashMap<&str, &str>) -> String {
    let mut out = String::new();
    // Each open section records whether its content is shown.
    let mut sections: Vec<bool> = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let visible = sections.iter().all(|&shown| shown);
        if visible {
            out.push_str(&rest[..start]);
        }
        let Some(len) = rest[start + 2..].find("}}") else {
            rest = &rest[start..];
            break;
        };
        let tag = rest[start + 2..start + 2 + len].trim();
        rest = &rest[start + 2 + len + 2..];

        let non_empty = |name: &str| fields.get(name.trim()).is_some_and(|v| !is_blank(v));
        if let Some(name) = tag.strip_prefix('#') {
            sections.push(non_empty(name));
        } else if let Some(name) = tag.strip_prefix('^') {
            sections.push(!non_empty(name));
        } else if tag.starts_with('/') {
            sections.pop();
        } else if visible {
            out.push_str(&field_reference(tag, fields));
        }
    }
    if sections.iter().all(|&shown| shown) {
        out.push_str(rest);
    }
    out
}

/// `Field` or `filter:filter:Field`.
fn field_reference(tag: &str, fields: &HashMap<&str, &str>) -> String {
    let mut parts = tag.rsplit(':');
    let name = parts.next().unwrap_or_default().trim();
    let value = fields.get(name).copied().unwrap_or_default();
    let mut value = value.to_string();
    // Filters apply right to left, innermost first.
    for filter in parts {
        let filter = filter.trim();
        value = match filter {
            "text" => strip_html(&value),
            // Typing boxes and text-to-speech have no Markdown equivalent.
            "cloze-only" => String::new(),
            f if f.starts_with("tts") || f.starts_with("type") => String::new(),
            "kanji" => strip_ruby(&value, false),
            "kana" => strip_ruby(&value, true),
            _ => value,
        };
    }
    value
}

/// Anki ruby syntax `日本[にほん] 語[ご]`: keep either the base text or the reading.
/// A space before a base only separates it and is dropped.
fn strip_ruby(s: &str, reading: bool) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| c + open) else { break };
        let (prefix, base) = match rest[..open].rfind(' ') {
            Some(space) => (&rest[..space], &rest[space + 1..open]),
            None => ("", &rest[..open]),
        };
        out.push_str(prefix);
        out.push_str(if reading { &rest[open + 1..close] } else { base });
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn is_blank(html: &str) -> bool {
    strip_html(html).trim().is_empty()
}

/// Plain text with tags removed and entities decoded.
pub fn strip_html(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..lt]));
        match rest[lt..].find('>') {
            Some(gt) => rest = &rest[lt + gt + 1..],
            None => {
                rest = &rest[lt..];
                break;
            }
        }
    }
    out.push_str(&decode_entities(rest));
    out
}

/// Media referenced while converting.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Media {
    /// Path from the deck file being written to the media folder, such as `../media/`.
    pub prefix: String,
    /// Anki media file names used by images.
    pub images: BTreeSet<String>,
    /// Anki media file names used by `[sound:…]` references.
    pub sounds: BTreeSet<String>,
}

/// Stands in for `[sound:…]` between finding sounds and reading tags.
const SOUND_TAG: &str = "amgi-sound";

/// Convert Anki field HTML to inline Markdown on as few lines as needed. Text is
/// escaped so it renders literally, including `[` and `]` (cloze markup in amgi).
/// Cloze markers `{{c1::…}}` pass through unchanged. Images and sounds link into the
/// media folder, both as `![](path)`; the app tells them apart by extension.
pub fn html_to_markdown(html: &str, media: &mut Media) -> String {
    let html = mark_sounds(html);
    let mut out = String::new();
    let mut in_code = false;
    // Raw-text elements whose content is never shown.
    let mut skip_until: Option<&str> = None;
    let mut rest = html.as_str();

    while !rest.is_empty() {
        let Some(lt) = rest.find('<') else {
            if skip_until.is_none() {
                push_text(&mut out, rest, in_code);
            }
            break;
        };
        if skip_until.is_none() {
            push_text(&mut out, &rest[..lt], in_code);
        }
        rest = &rest[lt..];
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.find("-->").map_or("", |end| &comment[end + 3..]);
            continue;
        }
        let Some(gt) = rest.find('>') else {
            // A lone `<` is text.
            if skip_until.is_none() {
                push_text(&mut out, rest, in_code);
            }
            break;
        };
        let tag = &rest[1..gt];
        rest = &rest[gt + 1..];

        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if let Some(until) = skip_until {
            if closing && name == until {
                skip_until = None;
            }
            continue;
        }
        match name.as_str() {
            "script" | "style" if !closing => {
                skip_until = Some(if name == "script" { "script" } else { "style" })
            }
            "br" | "hr" => out.push('\n'),
            "div" | "p" | "tr" | "ul" | "ol" | "table" | "blockquote" | "h1" | "h2" | "h3"
            | "h4" | "h5" | "h6" => out.push('\n'),
            "li" if !closing => out.push_str("\n- "),
            "td" | "th" if !closing => out.push(' '),
            "b" | "strong" => out.push_str("**"),
            "i" | "em" => out.push('*'),
            "code" | "kbd" => {
                out.push('`');
                in_code = !closing;
            }
            "img" => {
                if let Some(image) = attribute(tag, "src").and_then(|src| media_markdown(&src, &mut media.images, &media.prefix)) {
                    out.push_str(&image);
                }
            }
            SOUND_TAG => {
                // Sounds are always files in the package.
                let sound = attribute(tag, "src")
                    .filter(|src| !src.contains("://"))
                    .and_then(|src| media_markdown(&src, &mut media.sounds, &media.prefix));
                if let Some(sound) = sound {
                    out.push_str(&sound);
                }
            }
            _ => {}
        }
    }
    tidy(&out)
}

/// Value of an HTML attribute, quoted or not, with entities decoded.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| i + from) {
        from = i + name.len();
        // Must be a whole attribute name followed by `=`.
        let before_ok = lower[..i].ends_with(|c: char| c.is_whitespace());
        let Some(rest) = lower[from..].trim_start().strip_prefix('=') else { continue };
        if !before_ok {
            continue;
        }
        let value_start = tag.len() - rest.trim_start().len();
        let value = &tag[value_start..];
        let raw = match value.chars().next()? {
            q @ ('"' | '\'') => &value[1..value[1..].find(q)? + 1],
            // `<img src=a.png/>`: the slash closes the tag.
            _ => value.split(char::is_whitespace).next()?.trim_end_matches('/'),
        };
        return Some(decode_entities(raw));
    }
    None
}

/// `![](media/name.png)` for a file in the package, recorded in `used`. Remote
/// files are linked as they are.
fn media_markdown(src: &str, used: &mut BTreeSet<String>, prefix: &str) -> Option<String> {
    // Angle brackets and newlines cannot be written in a Markdown link.
    if src.contains(['<', '>', '\n']) {
        return None;
    }
    let path = if src.contains("://") {
        src.to_string()
    } else {
        // Anki media is a flat folder; anything else could escape the collection.
        if src.is_empty() || src.starts_with('.') || src.contains(['/', '\\']) {
            return None;
        }
        used.insert(src.to_string());
        format!("{prefix}{src}")
    };
    Some(if path.contains([' ', '(', ')']) { format!("![](<{path}>)") } else { format!("![]({path})") })
}

/// Replace `[sound:file.mp3]` with a tag read like `<img>`, so the file name is
/// checked the same way.
fn mark_sounds(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("[sound:") {
        let Some(end) = rest[start..].find(']') else { break };
        out.push_str(&rest[..start]);
        let name = &rest[start + "[sound:".len()..start + end];
        let name = name.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
        out.push_str(&format!(r#"<{SOUND_TAG} src="{name}">"#));
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

fn push_text(out: &mut String, raw: &str, in_code: bool) {
    // HTML collapses all whitespace, including newlines in the source.
    let decoded = decode_entities(raw);
    let mut prev_space = out.ends_with([' ', '\n']);
    let chars: Vec<char> = decoded.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
            continue;
        }
        prev_space = false;
        if !in_code && needs_escape(c, chars.get(i.wrapping_sub(1)), chars.get(i + 1)) {
            out.push('\\');
        }
        out.push(c);
    }
}

fn needs_escape(c: char, before: Option<&char>, after: Option<&char>) -> bool {
    match c {
        '\\' | '*' | '`' | '[' | ']' | '~' => true,
        // `_` only emphasizes at word boundaries, so `snake_case` stays readable.
        '_' => !(before.is_some_and(|c| c.is_alphanumeric()) && after.is_some_and(|c| c.is_alphanumeric())),
        _ => false,
    }
}

/// Trim lines, drop blank ones (a blank line ends an amgi card) and escape lines
/// that would otherwise start a new card or read as tags.
fn tidy(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| match l.get(..2) {
            Some("Q:" | "A:" | "C:") => format!("{}\\{}", &l[..1], &l[1..]),
            _ if crate::parser::tag_line(l).is_some() => format!("\\{l}"),
            _ => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..].find(';').filter(|&n| n <= 10).and_then(|n| {
            let entity = &rest[1..1 + n];
            let c = match entity {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                "nbsp" => ' ',
                _ => {
                    let code = match entity.strip_prefix('#') {
                        Some(hex) if hex.starts_with(['x', 'X']) => u32::from_str_radix(&hex[1..], 16).ok(),
                        Some(dec) => dec.parse().ok(),
                        None => None,
                    };
                    char::from_u32(code?)?
                }
            };
            Some((c, n + 2))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Convert Anki cloze markers in Markdown from [`html_to_markdown`] to amgi
/// deletions. Returns the text and the Anki cloze number of each deletion, in order.
/// Nested clozes are flattened into their parent. Deletions whose number `keep`
/// rejects, such as suspended ones, become plain text.
pub fn convert_clozes(markdown: &str, keep: impl Fn(u32) -> bool) -> (String, Vec<u32>) {
    let mut out = String::new();
    let mut numbers = Vec::new();
    let mut rest = markdown;
    while let Some((start, number, body_start)) = find_cloze(rest) {
        let Some(body_len) = cloze_end(&rest[body_start..]) else { break };
        out.push_str(&rest[..start]);
        let body = &rest[body_start..body_start + body_len];
        let (answer, hint) = split_hint(body);
        if !keep(number) {
            out.push_str(&flatten_clozes(answer));
            rest = &rest[body_start + body_len + 2..];
            continue;
        }
        out.push('[');
        out.push_str(&escape_bar(&flatten_clozes(answer)));
        if let Some(hint) = hint.map(str::trim).filter(|h| !h.is_empty()) {
            out.push('|');
            out.push_str(&escape_bar(&flatten_clozes(hint)));
        }
        out.push(']');
        numbers.push(number);
        rest = &rest[body_start + body_len + 2..];
    }
    out.push_str(rest);
    (out, numbers)
}

/// Position of the next `{{cN::`, its number and where its body starts.
fn find_cloze(s: &str) -> Option<(usize, u32, usize)> {
    let mut from = 0;
    while let Some(i) = s[from..].find("{{c").map(|i| i + from) {
        let digits = s[i + 3..].bytes().take_while(u8::is_ascii_digit).count();
        let after = i + 3 + digits;
        if digits > 0 && s[after..].starts_with("::") {
            return Some((i, s[i + 3..after].parse().ok()?, after + 2));
        }
        from = i + 3;
    }
    None
}

/// Length of a cloze body up to its matching `}}`, skipping nested clozes.
fn cloze_end(body: &str) -> Option<usize> {
    let mut depth = 0;
    let mut i = 0;
    while i < body.len() {
        if body[i..].starts_with("{{") {
            depth += 1;
            i += 2;
        } else if body[i..].starts_with("}}") {
            if depth == 0 {
                return Some(i);
            }
            depth -= 1;
            i += 2;
        } else {
            i += body[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    None
}

/// Split `answer::hint` at the first top-level `::`.
fn split_hint(body: &str) -> (&str, Option<&str>) {
    let mut depth = 0;
    let mut split = None;
    let mut i = 0;
    while i < body.len() {
        let rest = &body[i..];
        if rest.starts_with("{{") {
            depth += 1;
            i += 2;
        } else if rest.starts_with("}}") {
            depth -= 1;
            i += 2;
        } else if depth == 0 && rest.starts_with("::") {
            split = Some(i);
            break;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    match split {
        Some(i) => (&body[..i], Some(&body[i + 2..])),
        None => (body, None),
    }
}

fn flatten_clozes(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some((start, _, body_start)) = find_cloze(rest) {
        let Some(len) = cloze_end(&rest[body_start..]) else { break };
        out.push_str(&rest[..start]);
        out.push_str(&flatten_clozes(split_hint(&rest[body_start..body_start + len]).0));
        rest = &rest[body_start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// `|` separates a hint in amgi; escape it unless already escaped or in code.
fn escape_bar(s: &str) -> String {
    let (mut out, mut escaped, mut in_code) = (String::new(), false, false);
    for c in s.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' if !in_code => escaped = true,
            '`' => in_code = !in_code,
            '|' if !in_code => out.push('\\'),
            _ => {}
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(html: &str) -> String {
        html_to_markdown(html, &mut Media::default())
    }

    #[test]
    fn templates() {
        let fields = HashMap::from([("Front", "hi"), ("Back", "<b>yo</b>"), ("Extra", "")]);
        assert_eq!(render_template("{{Front}}", &fields), "hi");
        assert_eq!(render_template("{{text:Back}}!", &fields), "yo!");
        assert_eq!(render_template("a{{#Extra}}[{{Extra}}]{{/Extra}}b", &fields), "ab");
        assert_eq!(render_template("{{^Extra}}none{{/Extra}}", &fields), "none");
        assert_eq!(render_template("{{#Front}}{{^Back}}x{{/Back}}y{{/Front}}", &fields), "y");
        assert_eq!(render_template("{{type:Back}}{{Missing}}", &fields), "");
        assert_eq!(render_template("{{kana:F}}|{{kanji:F}}", &HashMap::from([("F", "日本[にほん] 語[ご]")])), "にほんご|日本語");
    }

    #[test]
    fn html() {
        assert_eq!(md("one<br>two<div>three</div>"), "one\ntwo\nthree");
        assert_eq!(md("<b>bold</b> &amp; <i>it</i>&nbsp;&#x41;&#66;"), "**bold** & *it* AB");
        assert_eq!(md("a  \n b<br><br><br>c"), "a b\nc");
        assert_eq!(md("2*3 [x] snake_case _x_ <code>v[0]*2</code>"), r"2\*3 \[x\] snake_case \_x\_ `v[0]*2`");
        assert_eq!(md("<style>.x{}</style>Q: real<!-- c -->"), r"Q\: real");
        assert_eq!(md("x < y &bogus; z"), "x < y &bogus; z");
        assert_eq!(md("text<br>#hashtag"), "text\n\\#hashtag");
        let mut media = Media { prefix: "../media/".into(), ..Media::default() };
        let html = r#"<img src="a.png">hi [sound:a.mp3]<IMG class=x SRC='b &amp; c.jpg'/><img src=../x.png><img data-src=no>"#;
        assert_eq!(html_to_markdown(html, &mut media), "![](../media/a.png)hi ![](../media/a.mp3)![](<../media/b & c.jpg>)");
        assert_eq!(media.images.iter().collect::<Vec<_>>(), ["a.png", "b & c.jpg"]);
        assert_eq!(media.sounds.iter().collect::<Vec<_>>(), ["a.mp3"]);
        // Names are kept literally; ones that could leave the media folder are dropped.
        let html = r#"[sound:my "word" & x.mp3][sound:<x>.mp3][sound:../a.mp3][sound:https://x.org/a.mp3] [sound:"#;
        assert_eq!(html_to_markdown(html, &mut media), r#"![](<../media/my "word" & x.mp3>) \[sound:"#);
        assert_eq!(media.sounds.iter().collect::<Vec<_>>(), ["a.mp3", r#"my "word" & x.mp3"#]);
        assert_eq!(md("<img src=https://x.org/a.png>"), "![](https://x.org/a.png)");
    }

    #[test]
    fn clozes() {
        let (text, numbers) = convert_clozes(&md("The {{c1::capital}} of [x] is {{c2::Paris::city}}, {{c1::a|b}}."), |_| true);
        assert_eq!(text, r"The [capital] of \[x\] is [Paris|city], [a\|b].");
        assert_eq!(numbers, [1, 2, 1]);
        // A suspended deletion is no longer a gap.
        let (text, numbers) = convert_clozes("{{c1::a}} {{c2::b::hint}} {{c3::c}}", |n| n != 2);
        assert_eq!((text.as_str(), numbers), ("[a] b [c]", vec![1, 3]));
        let (text, numbers) = convert_clozes("{{c1::outer {{c2::inner::h}} end}} {{c3::x::y::z}} {{c1::open", |_| true);
        assert_eq!(text, "[outer inner end] [x|y::z] {{c1::open");
        assert_eq!(numbers, [1, 3]);
    }
}
