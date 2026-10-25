//! Raw HTML images enter the same bounded asset path as authored Markdown images.
//! Only src/alt are retained; executable attributes never reach a renderer.

/// Normalize supported img tags, leaving code examples and malformed tags literal.
pub(crate) fn normalize(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut fence: Option<(u8, usize)> = None;
    let mut opaque: Option<&str> = None;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.as_bytes().first().copied();
        let count = marker.map_or(0, |marker| {
            trimmed.bytes().take_while(|byte| *byte == marker).count()
        });
        if matches!(marker, Some(b'`' | b'~')) && count >= 3 {
            if let Some((byte, length)) = fence {
                if marker == Some(byte) && count >= length && trimmed[count..].trim().is_empty() {
                    fence = None;
                }
            } else {
                fence = marker.map(|byte| (byte, count));
            }
            output.push_str(line);
            continue;
        }
        if fence.is_some() {
            output.push_str(line);
            continue;
        }
        let mut offset = 0;
        let mut code = 0;
        while offset < line.len() {
            let rest = &line[offset..];
            if let Some(end) = opaque {
                if rest
                    .get(..end.len())
                    .is_some_and(|text| text.eq_ignore_ascii_case(end))
                {
                    output.push_str(&rest[..end.len()]);
                    offset += end.len();
                    opaque = None;
                } else {
                    let character = rest.chars().next().unwrap();
                    output.push(character);
                    offset += character.len_utf8();
                }
                continue;
            }
            if code == 0 {
                for (start, end) in [
                    ("<!--", "-->"),
                    ("<script", "</script>"),
                    ("<style", "</style>"),
                ] {
                    if rest
                        .get(..start.len())
                        .is_some_and(|text| text.eq_ignore_ascii_case(start))
                        && (start == "<!--"
                            || rest
                                .as_bytes()
                                .get(start.len())
                                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>'))
                    {
                        opaque = Some(end);
                        break;
                    }
                }
                if opaque.is_some() {
                    continue;
                }
            }
            if rest.starts_with('`') {
                let count = rest.bytes().take_while(|byte| *byte == b'`').count();
                if code == 0 {
                    code = count;
                } else if code == count {
                    code = 0;
                }
                output.push_str(&rest[..count]);
                offset += count;
                continue;
            }
            if code == 0
                && rest
                    .get(..4)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("<img"))
                && rest.as_bytes().get(4).is_some_and(|byte| {
                    byte.is_ascii_whitespace() || *byte == b'/' || *byte == b'>'
                })
                && let Some((length, src, alt)) = image_tag(rest)
            {
                // Angle-delimited destinations support spaces; brackets in authored alt
                // text cannot inject a second Markdown node.
                if !src.contains(['<', '>', '\n', '\r']) {
                    output.push_str(&format!("![{}](<{}>)", alt.replace(['[', ']'], ""), src));
                    offset += length;
                    continue;
                }
            }
            let character = rest.chars().next().unwrap();
            output.push(character);
            offset += character.len_utf8();
        }
    }
    output
}

fn image_tag(input: &str) -> Option<(usize, String, String)> {
    let mut offset = 4;
    let mut src = None;
    let mut alt = String::new();
    while offset < input.len() {
        while input
            .as_bytes()
            .get(offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            offset += 1;
        }
        if input[offset..].starts_with("/>") {
            offset += 1;
        }
        if input[offset..].starts_with('>') {
            return src.map(|src| (offset + 1, src, alt));
        }
        let start = offset;
        while input
            .as_bytes()
            .get(offset)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
        {
            offset += 1;
        }
        if start == offset {
            return None;
        }
        let name = &input[start..offset];
        while input
            .as_bytes()
            .get(offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            offset += 1;
        }
        if input.as_bytes().get(offset) != Some(&b'=') {
            continue;
        }
        offset += 1;
        while input
            .as_bytes()
            .get(offset)
            .is_some_and(u8::is_ascii_whitespace)
        {
            offset += 1;
        }
        let quote = input.as_bytes().get(offset).copied()?;
        let value = if matches!(quote, b'\'' | b'"') {
            offset += 1;
            let start = offset;
            let end = input[offset..].find(char::from(quote))? + offset;
            offset = end + 1;
            &input[start..end]
        } else {
            let start = offset;
            while input
                .as_bytes()
                .get(offset)
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'>')
            {
                offset += 1;
            }
            &input[start..offset]
        };
        // Decode only common attribute entities. Unknown entities remain literal paths.
        let value = value
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        if name.eq_ignore_ascii_case("src") && src.is_none() {
            src = Some(value);
        } else if name.eq_ignore_ascii_case("alt") {
            alt = value;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r18_d_html_images_retain_exact_paths_and_strip_executable_attributes() {
        assert_eq!(
            normalize("before <IMG src='charts/my plot.png' alt='résultat' onerror='bad()'> after"),
            "before ![résultat](<charts/my plot.png>) after"
        );
        assert_eq!(
            normalize("<img src=chart.png alt='a &amp; b'/>"),
            "![a & b](<chart.png>)"
        );
    }
    #[test]
    fn r18_d_html_examples_and_malformed_tags_remain_literal() {
        let input = "````html\n<img src='example.png'>\n```\n<img src='still-example.png'>\n````\n`<img src='inline.png'>`\n<img src='unfinished\n";
        assert_eq!(normalize(input), input);
        let hidden = "<!--\n<img src='comment.png'>\n-->\n<script><img src='script.png'></script>\n<style><img src='style.png'></style>";
        assert_eq!(normalize(hidden), hidden);
        assert_eq!(
            normalize("<image src='not-image.png'>"),
            "<image src='not-image.png'>"
        );
    }
    #[test]
    fn r18_d_html_unsafe_destinations_cannot_inject_markdown() {
        assert_eq!(
            normalize("<img src='a&gt;b.png'>"),
            "<img src='a&gt;b.png'>"
        );
        let (text, images) = crate::conversation_presentation::workspace_markdown_images(
            "<img src='javascript:bad()' alt='unsafe'> <img src='data:image/png;base64,AAAA'>",
        );
        assert!(images.is_empty());
        assert!(!text.contains("!["));
    }
}
