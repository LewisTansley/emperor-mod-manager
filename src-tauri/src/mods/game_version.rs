//! Compare an installed game with the game line a mod actually names.
//!
//! Nexus has no game-version field. Tags, the staged file name, and collection
//! versions are evidence only when they name a line that breaks mods. A missing
//! tag, a mod's own version, or a different patch of the same line is not a warning.

use serde::Serialize;

/// Bump when the cached tag list must be fetched again.
/// Generation 0 is the old dotted-only cache, which dropped labels such as `AE`.
pub const VERSION_SUPPORT_GENERATION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModVersionStatus {
    Ok,
    Mismatch,
    /// Older checks used this when a mod named no version. New checks stay quiet instead.
    #[allow(dead_code)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModVersionAdvice {
    pub status: ModVersionStatus,
    pub supported: Vec<String>,
    pub message: String,
}

pub struct AdviseInput<'a> {
    pub domain: &'a str,
    pub current: Option<&'a str>,
    pub tags: &'a [String],
    pub file_name: Option<&'a str>,
    pub mod_version: Option<&'a str>,
    pub collection_versions: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GameLine {
    key: String,
    nickname: Option<String>,
    built_label: String,
}

/// Warn only when evidence names a different line than the install.
/// No evidence, or an install that is not itself a line, is compatible.
pub fn advise(input: &AdviseInput<'_>) -> ModVersionAdvice {
    let Some(current) = input.current.map(str::trim).filter(|s| !s.is_empty()) else {
        return compatible(Vec::new());
    };
    let Some(install_line) = classify_install(input.domain, current) else {
        return compatible(Vec::new());
    };

    let mut lines = Vec::new();
    for token in input.tags {
        if let Some(line) = classify_loose(input.domain, token, current, input.mod_version) {
            push_line(&mut lines, line);
        }
    }
    if let Some(file_name) = input.file_name {
        for token in file_tokens(file_name) {
            if let Some(line) = classify_loose(input.domain, &token, current, input.mod_version) {
                push_line(&mut lines, line);
            }
        }
    }
    for token in input.collection_versions {
        if let Some(line) = classify_collection(input.domain, token) {
            push_line(&mut lines, line);
        }
    }

    let supported = lines.iter().map(|line| line.built_label.clone()).collect();
    if lines.is_empty() || lines.iter().any(|line| line.key == install_line.key) {
        return compatible(supported);
    }
    let built = lines
        .iter()
        .map(|line| line.built_label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    ModVersionAdvice {
        status: ModVersionStatus::Mismatch,
        supported,
        message: format!(
            "Built for {built}. This install is {}.",
            install_phrase(current, &install_line)
        ),
    }
}

/// True when a Nexus tag is an edition label (`AE`, `Next-Gen`) for this game.
pub fn is_line_alias(domain: &str, tag: &str) -> bool {
    alias_line(domain, tag).is_some()
}

fn compatible(supported: Vec<String>) -> ModVersionAdvice {
    ModVersionAdvice {
        status: ModVersionStatus::Ok,
        supported,
        message: String::new(),
    }
}

fn classify_install(domain: &str, current: &str) -> Option<GameLine> {
    let parts = numeric_segments(current)?;
    if known_table(domain) {
        return classify_numbers(domain, &parts);
    }
    generic_line(&parts)
}

fn classify_loose(
    domain: &str,
    token: &str,
    current: &str,
    mod_version: Option<&str>,
) -> Option<GameLine> {
    if same_as_mod_version(token, mod_version) {
        return None;
    }
    if let Some(line) = alias_line(domain, token) {
        return Some(line);
    }
    let parts = numeric_segments(token)?;
    if known_table(domain) {
        return classify_numbers(domain, &parts);
    }
    let install = numeric_segments(current)?;
    if install.first() != parts.first() {
        return None;
    }
    generic_line(&parts)
}

fn classify_collection(domain: &str, token: &str) -> Option<GameLine> {
    if let Some(line) = alias_line(domain, token) {
        return Some(line);
    }
    let parts = numeric_segments(token)?;
    if known_table(domain) {
        classify_numbers(domain, &parts)
    } else {
        generic_line(&parts)
    }
}

fn known_table(domain: &str) -> bool {
    matches!(
        normalize_domain(domain).as_str(),
        "skyrimspecialedition" | "fallout4"
    )
}

fn classify_numbers(domain: &str, parts: &[u64]) -> Option<GameLine> {
    match normalize_domain(domain).as_str() {
        "skyrimspecialedition" => skyrim_line(parts),
        "fallout4" => fallout4_line(parts),
        _ => generic_line(parts),
    }
}

fn skyrim_line(parts: &[u64]) -> Option<GameLine> {
    if parts.len() < 2 || parts[0] != 1 {
        return None;
    }
    match parts[1] {
        5 => Some(line("1.5", Some("SE"), "1.5 (SE)")),
        6 => Some(line("1.6", Some("AE"), "1.6 (AE)")),
        _ => None,
    }
}

fn fallout4_line(parts: &[u64]) -> Option<GameLine> {
    if parts.len() < 3 || parts[0] != 1 || parts[1] != 10 {
        return None;
    }
    if parts[2] <= 163 {
        Some(line("old-gen", Some("old-gen"), "old-gen"))
    } else if parts[2] >= 980 {
        Some(line("next-gen", Some("next-gen"), "next-gen"))
    } else {
        None
    }
}

fn generic_line(parts: &[u64]) -> Option<GameLine> {
    if parts.len() < 2 {
        return None;
    }
    let label = format!("{}.{}", parts[0], parts[1]);
    Some(line(&label, None, &label))
}

fn alias_line(domain: &str, token: &str) -> Option<GameLine> {
    let key = alias_key(token);
    match normalize_domain(domain).as_str() {
        "skyrimspecialedition" => match key.as_str() {
            "se" => Some(line("1.5", Some("SE"), "1.5 (SE)")),
            "ae" | "anniversary" | "anniversaryedition" => {
                Some(line("1.6", Some("AE"), "1.6 (AE)"))
            }
            _ => None,
        },
        "fallout4" => match key.as_str() {
            "oldgen" => Some(line("old-gen", Some("old-gen"), "old-gen")),
            "nextgen" | "ng" => Some(line("next-gen", Some("next-gen"), "next-gen")),
            _ => None,
        },
        _ => None,
    }
}

fn line(key: &str, nickname: Option<&str>, built_label: &str) -> GameLine {
    GameLine {
        key: key.to_string(),
        nickname: nickname.map(str::to_string),
        built_label: built_label.to_string(),
    }
}

fn install_phrase(current: &str, line: &GameLine) -> String {
    match &line.nickname {
        Some(name) => format!("{current} ({name})"),
        None => current.to_string(),
    }
}

fn push_line(lines: &mut Vec<GameLine>, line: GameLine) {
    if lines.iter().any(|existing| existing.key == line.key) {
        return;
    }
    lines.push(line);
}

fn same_as_mod_version(token: &str, mod_version: Option<&str>) -> bool {
    let Some(mod_version) = mod_version.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if token.trim().eq_ignore_ascii_case(mod_version) {
        return true;
    }
    match (numeric_segments(token), numeric_segments(mod_version)) {
        (Some(token_parts), Some(mod_parts)) => {
            strip_trailing_zeros(token_parts) == strip_trailing_zeros(mod_parts)
        }
        _ => false,
    }
}

fn strip_trailing_zeros(mut parts: Vec<u64>) -> Vec<u64> {
    while parts.last() == Some(&0) {
        parts.pop();
    }
    parts
}

fn numeric_segments(value: &str) -> Option<Vec<u64>> {
    let value = value.trim().trim_start_matches(['v', 'V']);
    if !value.contains('.') {
        return None;
    }
    let mut parts = Vec::new();
    for part in value.split('.') {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse::<u64>().ok()?);
    }
    if parts.len() < 2 {
        return None;
    }
    Some(parts)
}

fn file_tokens(name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = name.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            let mut saw_dot = false;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                if chars[i] == '.' {
                    saw_dot = true;
                }
                i += 1;
            }
            if saw_dot {
                let token: String = chars[start..i].iter().collect();
                let token = token.trim_matches('.').to_string();
                if numeric_segments(&token).is_some() {
                    push_token(&mut out, token);
                }
            }
            continue;
        }
        i += 1;
    }
    let mut word = String::new();
    for c in name.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphabetic() {
            word.push(c);
        } else if !word.is_empty() {
            push_token(&mut out, word.clone());
            word.clear();
        }
    }
    out
}

fn push_token(out: &mut Vec<String>, token: String) {
    if token.is_empty() || out.iter().any(|existing| existing == &token) {
        return;
    }
    out.push(token);
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().to_ascii_lowercase()
}

fn alias_key(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    fn advise_for(
        domain: &str,
        current: Option<&str>,
        tag_values: &[&str],
        file_name: Option<&str>,
        mod_version: Option<&str>,
        collections: &[&str],
    ) -> ModVersionAdvice {
        let tag_list = tags(tag_values);
        let collection_versions = tags(collections);
        advise(&AdviseInput {
            domain,
            current,
            tags: &tag_list,
            file_name,
            mod_version,
            collection_versions: &collection_versions,
        })
    }

    #[test]
    fn no_evidence_is_compatible() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &[],
            None,
            None,
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
        assert!(advice.message.is_empty());
    }

    #[test]
    fn same_skyrim_line_ignores_the_patch() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &["1.6.640"],
            None,
            None,
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }

    #[test]
    fn skyrim_se_evidence_warns_on_an_ae_install() {
        for token in ["1.5.97", "SE"] {
            let advice = advise_for(
                "skyrimspecialedition",
                Some("1.6.1170"),
                &[token],
                None,
                None,
                &[],
            );
            assert_eq!(advice.status, ModVersionStatus::Mismatch, "{token}");
            assert_eq!(
                advice.message,
                "Built for 1.5 (SE). This install is 1.6.1170 (AE)."
            );
        }
    }

    #[test]
    fn a_collection_on_another_patch_of_the_same_line_does_not_warn() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &[],
            None,
            None,
            &["1.6.640"],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }

    #[test]
    fn a_collection_on_another_line_warns() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &[],
            None,
            None,
            &["1.5.97"],
        );
        assert_eq!(advice.status, ModVersionStatus::Mismatch);
    }

    #[test]
    fn skyrim_drops_tags_that_are_not_a_game_line() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &["1.0", "2.1"],
            None,
            None,
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }

    #[test]
    fn file_name_can_name_the_line() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &[],
            Some("SkyUI_5_2_SE"),
            Some("5.2"),
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Mismatch);
        assert_eq!(
            advice.message,
            "Built for 1.5 (SE). This install is 1.6.1170 (AE)."
        );
    }

    #[test]
    fn evidence_that_includes_the_install_line_is_compatible() {
        let advice = advise_for(
            "skyrimspecialedition",
            Some("1.6.1170"),
            &["1.5.97", "AE"],
            None,
            None,
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }

    #[test]
    fn other_games_warn_on_a_different_minor() {
        let advice = advise_for("cyberpunk2077", Some("2.2.1"), &["2.1"], None, None, &[]);
        assert_eq!(advice.status, ModVersionStatus::Mismatch);
        assert_eq!(advice.message, "Built for 2.1. This install is 2.2.1.");
    }

    #[test]
    fn other_games_ignore_a_different_patch_and_a_different_major() {
        let same_minor = advise_for("cyberpunk2077", Some("2.2.1"), &["2.2.0"], None, None, &[]);
        assert_eq!(same_minor.status, ModVersionStatus::Ok);
        let other_major = advise_for("cyberpunk2077", Some("2.2.1"), &["1.0"], None, None, &[]);
        assert_eq!(other_major.status, ModVersionStatus::Ok);
    }

    #[test]
    fn a_token_equal_to_the_mod_version_is_not_evidence() {
        let advice = advise_for(
            "cyberpunk2077",
            Some("2.2.1"),
            &["2.1"],
            None,
            Some("2.1.0"),
            &[],
        );
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }

    #[test]
    fn fallout4_next_gen_is_its_own_line() {
        let advice = advise_for("fallout4", Some("1.10.984"), &["1.10.163"], None, None, &[]);
        assert_eq!(advice.status, ModVersionStatus::Mismatch);
        assert_eq!(
            advice.message,
            "Built for old-gen. This install is 1.10.984 (next-gen)."
        );
    }

    #[test]
    fn does_not_claim_a_mismatch_when_the_install_version_is_unknown() {
        let advice = advise_for("skyrimspecialedition", None, &["1.5"], None, None, &[]);
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }
}
