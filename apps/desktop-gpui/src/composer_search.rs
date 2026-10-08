//! Composer ranking follows the shared web search's tiered score and subsequence penalties.
fn score(
    value: &str,
    query: &str,
    base: usize,
    fuzzy: Option<usize>,
    markers: &[char],
) -> Option<usize> {
    let value = value.to_lowercase();
    if value.is_empty() || query.is_empty() {
        return None;
    }
    let length = value
        .chars()
        .count()
        .saturating_sub(query.chars().count())
        .min(64);
    if value == query {
        return Some(base);
    }
    if value.starts_with(query) {
        return Some(base + 2 + length);
    }
    let boundary = markers
        .iter()
        .filter_map(|marker| {
            value
                .find(&format!("{marker}{query}"))
                .map(|i| value[..i].chars().count() + 1)
        })
        .min();
    if let Some(i) = boundary {
        return Some(base + 4 + i * 2 + length);
    }
    if let Some(i) = value.find(query) {
        return Some(base + 6 + value[..i].chars().count() * 2 + length);
    }
    let fuzzy = fuzzy?;
    let needle: Vec<_> = query.chars().collect();
    let mut matched = 0;
    let mut first = 0;
    let mut previous = 0;
    let mut gaps = 0;
    for (i, ch) in value.chars().enumerate() {
        if ch != needle[matched] {
            continue;
        }
        if matched == 0 {
            first = i;
        } else {
            gaps += i - previous - 1;
        }
        previous = i;
        matched += 1;
        if matched == needle.len() {
            return Some(fuzzy + first * 2 + gaps * 3 + i - first + 1 - needle.len() + length);
        }
    }
    None
}

pub fn command_score(label: &str, description: &str, query: &str) -> Option<usize> {
    let query = query.trim().trim_start_matches(['/', '$']).to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let name = label.trim_start_matches(['/', '$']);
    let skill = name.starts_with("skill:") || label.starts_with('$');
    let name = name.strip_prefix("skill:").unwrap_or(name);
    let query = if skill {
        query.strip_prefix("skill:").unwrap_or(&query)
    } else {
        &query
    };
    if skill && (query.is_empty() || query == "skill") {
        return Some(0);
    }
    let result = [
        score(name, query, 0, Some(100), &['-', '_', '/']),
        score(description, query, 20, None, &[' ', '-', '_', '/']),
    ]
    .into_iter()
    .flatten()
    .min();
    result.or_else(|| (skill && "skill".starts_with(query)).then_some(usize::MAX))
}

pub fn skill_score(skill: &serde_json::Value, query: &str) -> Option<usize> {
    let normalized = query.trim().trim_start_matches(['/', '$']).to_lowercase();
    if normalized.is_empty() || normalized == "skill" {
        return Some(0);
    }
    let query = normalized.strip_prefix("skill:").unwrap_or(&normalized);
    if query.is_empty() {
        return Some(0);
    }
    let field = |key: &str| skill[key].as_str().unwrap_or_default();
    let scores = [
        score(field("name"), query, 0, Some(100), &['-', '_', '/']),
        score(
            field("displayName"),
            query,
            1,
            Some(110),
            &[' ', '-', '_', '/'],
        ),
        score(
            field("shortDescription"),
            query,
            20,
            None,
            &[' ', '-', '_', '/'],
        ),
        score(field("description"), query, 30, None, &[' ', '-', '_', '/']),
        score(field("scope"), query, 40, None, &[]),
    ];
    scores
        .into_iter()
        .flatten()
        .min()
        .or_else(|| "skill".starts_with(query).then_some(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn r14_c_command_fuzzy_boundary_and_description_ranking() {
        let mut items = [
            "/build-ui",
            "/ui",
            "/gh-fix-ci",
            "/skill:browser-control",
            "/audit",
        ];
        items.sort_by_key(|name| command_score(name, "", "ui").unwrap_or(usize::MAX));
        assert_eq!(&items[..2], &["/ui", "/build-ui"]);
        assert!(command_score("/gh-fix-ci", "", "gfc").is_some());
        assert_eq!(
            command_score("/skill:browser-control", "", "/skill:brow"),
            command_score("/skill:browser-control", "", "brow")
        );
        assert!(command_score("/skill:browser", "", "/sk").is_some());
        assert!(command_score("/skill:browser", "", "/ill").is_none());
        assert!(command_score("/audit", "UI checks", "ui").is_some());
        assert!(command_score("/audit", "generic words", "gw").is_none());
        let skill = serde_json::json!({"name":"ask-matt","displayName":"Ask Matt", "description":"Get advice"});
        assert!(skill_score(&skill, "ask matt").is_some());
        assert!(skill_score(&skill, "am").is_some());
        assert!(skill_score(&skill, "ga").is_none());
    }
}
