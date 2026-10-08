//! A completed, unambiguous shell fence may hand off to H's scoped terminal API.
/// GPUI's Markdown AST omits the terminal newline from code. Recover it only
/// when the source span proves a closed fence and exactly matches the display.
pub(super) fn completed_code(
    source: &str,
    span: Option<std::ops::Range<usize>>,
    code: &str,
) -> Option<String> {
    let block = source.get(span?)?;
    let (opening, rest) = block.split_once('\n')?;
    let opening = opening.trim_start();
    let marker = opening.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = opening.chars().take_while(|ch| *ch == marker).count();
    let (body, closing) = rest.trim_end_matches('\n').rsplit_once('\n')?;
    let closing = closing.trim();
    if length < 3
        || closing.chars().count() < length
        || !closing.chars().all(|ch| ch == marker)
        || body.trim_end_matches('\n') != code.trim_end_matches('\n')
    {
        return None;
    }
    Some(format!("{body}\n"))
}

pub(super) fn runnable(language: &str, code: &str, streaming: bool) -> bool {
    !streaming
        && matches!(
            language,
            "sh" | "bash" | "zsh" | "fish" | "shell" | "powershell" | "pwsh"
        )
        && code.ends_with('\n')
        && !code.trim().is_empty()
        && !code.trim_end().ends_with('\\')
        && !code[..code.len() - 1]
            .chars()
            .any(|ch| ch.is_control() || format_character(ch))
}

fn format_character(ch: char) -> bool {
    matches!(ch as u32, 0x00ad | 0x0600..=0x0605 | 0x061c | 0x06dd | 0x070f | 0x0890..=0x0891 | 0x08e2 | 0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0x2066..=0x206f | 0xfeff | 0xfff9..=0xfffb | 0x110bd | 0x110cd | 0x13430..=0x1343f | 0x1bca0..=0x1bca3 | 0x1d173..=0x1d17a | 0xe0001 | 0xe0020..=0xe007f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r13_d_shell_source_span_must_prove_a_closed_matching_fence() {
        let source = "```bash\necho hi\n```";
        assert_eq!(
            completed_code(source, Some(0..source.len()), "echo hi"),
            Some("echo hi\n".into())
        );
        let open = "```bash\necho hi";
        assert!(completed_code(open, Some(0..open.len()), "echo hi").is_none());
        assert!(completed_code(source, Some(0..source.len()), "different").is_none());
        assert!(completed_code(source, None, "echo hi").is_none());
    }
    #[test]
    fn r13_d_shell_fence_requires_settled_complete_visible_command() {
        assert!(runnable("bash", "echo café\n", false));
        for code in [
            "",
            "echo hi",
            "echo \\\n",
            "echo hi\nrm file\n",
            "echo \u{202e}hi\n",
            "echo \u{00ad}hi\n",
        ] {
            assert!(!runnable("bash", code, false), "{code:?}");
        }
        assert!(!runnable("rust", "echo hi\n", false));
        assert!(!runnable("bash", "echo hi\n", true));
    }
}
