//! KakaoTalk RichText uses UTF-16 [location, length] pairs.
//! Only supported inline styles are translated; other Markdown stays literal.
use std::{collections::BTreeMap, ops::Range};

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::Serialize;

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Styles {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    bold: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    italic: Vec<[usize; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    strike: Vec<[usize; 2]>,
}

#[derive(Debug, PartialEq)]
pub struct RichTextMessage {
    pub text: String,
    styles: Styles,
}

impl RichTextMessage {
    pub fn attachment(&self) -> String {
        serde_json::json!({"richText": self.styles}).to_string()
    }

    pub fn from_markdown(source: &str) -> Self {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        let mut spans = Vec::new();
        let mut code_block = false;
        let mut leaf_end = 0;
        for (event, range) in
            Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH).into_offset_iter()
        {
            let style = match event {
                Event::Start(Tag::Strong) => Some((0, 2)),
                Event::Start(Tag::Emphasis) => Some((1, 1)),
                Event::Start(Tag::Strikethrough) => Some((
                    2,
                    if source[range.clone()].starts_with("~~") {
                        2
                    } else {
                        1
                    },
                )),
                Event::Start(Tag::CodeBlock(_)) => {
                    code_block = true;
                    None
                }
                Event::End(TagEnd::CodeBlock) => {
                    code_block = false;
                    None
                }
                // CommonMark resolves escapes and entities outside literal code.
                Event::Text(text) if !code_block => {
                    // The parser excludes an escape's backslash from the text range.
                    if range.start < range.end
                        && range.start > leaf_end
                        && source.as_bytes()[range.start - 1] == b'\\'
                        && source.as_bytes()[range.start].is_ascii_punctuation()
                    {
                        edits.push((range.start - 1..range.start, String::new()));
                    }
                    if source[range.clone()] != *text {
                        edits.push((range.clone(), text.into_string()));
                    }
                    leaf_end = range.end;
                    None
                }
                Event::Start(_) | Event::End(_) => None,
                _ => {
                    leaf_end = range.end;
                    None
                }
            };
            if let Some((style, width)) = style {
                edits.push((range.start..range.start + width, String::new()));
                edits.push((range.end - width..range.end, String::new()));
                spans.push((style, range.start + width..range.end - width));
            }
        }
        edits.sort_by_key(|(range, _)| range.start);
        let mut text = String::with_capacity(source.len());
        let mut offsets = BTreeMap::new();
        let mut cursor = 0;
        let mut utf16 = 0;
        for (range, replacement) in edits {
            let literal = &source[cursor..range.start];
            text.push_str(literal);
            utf16 += literal.encode_utf16().count();
            offsets.insert(range.start, utf16);
            text.push_str(&replacement);
            utf16 += replacement.encode_utf16().count();
            offsets.insert(range.end, utf16);
            cursor = range.end;
        }
        text.push_str(&source[cursor..]);
        let mut styles = Styles::default();
        for (style, range) in spans {
            let start = offsets[&range.start];
            let end = offsets[&range.end];
            if end > start {
                let ranges = match style {
                    0 => &mut styles.bold,
                    1 => &mut styles.italic,
                    _ => &mut styles.strike,
                };
                ranges.push([start, end - start]);
            }
        }
        for ranges in [&mut styles.bold, &mut styles.italic, &mut styles.strike] {
            normalize(ranges);
        }
        Self { text, styles }
    }
}

fn normalize(ranges: &mut Vec<[usize; 2]>) {
    ranges.sort_unstable();
    let mut merged: Vec<[usize; 2]> = Vec::new();
    for [start, length] in ranges.drain(..) {
        if let Some(previous) = merged.last_mut()
            && start <= previous[0] + previous[1]
        {
            previous[1] = (start + length).max(previous[0] + previous[1]) - previous[0];
        } else {
            merged.push([start, length]);
        }
    }
    *ranges = merged;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn check(input: &str, text: &str, styles: Value) {
        let message = RichTextMessage::from_markdown(input);
        assert_eq!(message.text, text);
        assert_eq!(
            serde_json::from_str::<Value>(&message.attachment()).unwrap(),
            json!({"richText": styles})
        );
    }

    #[test]
    fn translates_three_native_styles() {
        check(
            "**굵게** *기울임* ~~취소~~",
            "굵게 기울임 취소",
            json!({"bold":[[0,2]],"italic":[[3,3]],"strike":[[7,2]]}),
        );
        check(
            "__bold__ _italic_",
            "bold italic",
            json!({"bold":[[0,4]],"italic":[[5,6]]}),
        );
    }

    #[test]
    fn measures_utf16_not_bytes_or_codepoints() {
        check(
            "😀 **한👨‍👩‍👧‍👦글** *e\u{301}*",
            "😀 한👨‍👩‍👧‍👦글 e\u{301}",
            json!({"bold":[[3,13]],"italic":[[17,2]]}),
        );
    }

    #[test]
    fn nested_and_repeated_styles() {
        check(
            "***both*** ~~**gone**~~",
            "both gone",
            json!({"bold":[[0,4],[5,4]],"italic":[[0,4]],"strike":[[5,4]]}),
        );
        check("**a __b__ c**", "a b c", json!({"bold":[[0,5]]}));
        check("*a* and *b*", "a and b", json!({"italic":[[0,1],[6,1]]}));
    }

    #[test]
    fn resolves_escapes_and_entities_without_styling_literals() {
        check(
            r"\*literal\* **a &amp; b**",
            "*literal* a & b",
            json!({"bold":[[10,5]]}),
        );
        check(
            "`**code**` **ok**",
            "`**code**` ok",
            json!({"bold":[[11,2]]}),
        );
        check(
            "```\n**code** &amp;\n```\n**ok**",
            "```\n**code** &amp;\n```\nok",
            json!({"bold":[[23,2]]}),
        );
    }

    #[test]
    fn preserves_unsupported_markdown_and_whitespace() {
        check(
            "# title\n\n- **item**\n> quote\n[link](https://example.org)\n",
            "# title\n\n- item\n> quote\n[link](https://example.org)\n",
            json!({"bold":[[11,4]]}),
        );
        check("plain\n\n**unclosed", "plain\n\n**unclosed", json!({}));
        check("", "", json!({}));
    }
    #[test]
    fn malformed_and_unicode_inputs_keep_valid_ranges() {
        let tokens = [
            "*", "**", "_", "__", "~", "~~", "`", "\\", "[", "]", "(", ")", "&amp;", " ", "\n",
            "한", "😀", "a",
        ];
        let mut seed = 42u64;
        for _ in 0..4096 {
            let mut input = String::new();
            for _ in 0..32 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                input.push_str(tokens[(seed >> 32) as usize % tokens.len()]);
            }
            let message = RichTextMessage::from_markdown(&input);
            let mut boundaries = vec![0];
            let mut offset = 0;
            for ch in message.text.chars() {
                offset += ch.len_utf16();
                boundaries.push(offset);
            }
            for ranges in [
                &message.styles.bold,
                &message.styles.italic,
                &message.styles.strike,
            ] {
                for &[start, length] in ranges {
                    assert!(
                        length > 0
                            && boundaries.contains(&start)
                            && boundaries.contains(&(start + length)),
                        "{input:?}"
                    );
                }
            }
        }
    }
}
