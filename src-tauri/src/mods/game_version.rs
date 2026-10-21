//! Compare an installed game version with a mod's official support.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModVersionStatus {
    Ok,
    Mismatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModVersionAdvice {
    pub status: ModVersionStatus,
    pub supported: Vec<String>,
    pub message: String,
}

/// `tags` are version-like Nexus tags. `collection_versions` come from installed
/// collections that include the mod. An empty list means that source said nothing.
pub fn advise(
    current: Option<&str>,
    tags: &[String],
    collection_versions: &[String],
) -> ModVersionAdvice {
    let supported = unique_versions(tags, collection_versions);
    if tags.is_empty() && collection_versions.is_empty() {
        return ModVersionAdvice {
            status: ModVersionStatus::Unknown,
            supported,
            message: "Could not detect supported game version".into(),
        };
    }
    let Some(current) = current.map(str::trim).filter(|s| !s.is_empty()) else {
        return ModVersionAdvice {
            status: ModVersionStatus::Ok,
            supported,
            message: String::new(),
        };
    };
    let tags_ok = tags.is_empty() || tags.iter().any(|v| version_matches(v, current));
    let collections_ok = collection_versions.is_empty()
        || collection_versions
            .iter()
            .any(|v| version_matches(v, current));
    if tags_ok && collections_ok {
        ModVersionAdvice {
            status: ModVersionStatus::Ok,
            supported,
            message: String::new(),
        }
    } else {
        ModVersionAdvice {
            status: ModVersionStatus::Mismatch,
            supported,
            message: format!("Does not officially support {current}"),
        }
    }
}

fn unique_versions(tags: &[String], collection_versions: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for value in tags.iter().chain(collection_versions.iter()) {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(value))
        {
            out.push(value.to_string());
        }
    }
    out
}

/// A shorter official version matches a longer install when it is a prefix.
/// A trailing `.0` is ignored. A different patch does not match.
pub fn version_matches(official: &str, installed: &str) -> bool {
    let official = official.trim();
    let installed = installed.trim();
    if official.is_empty() || installed.is_empty() {
        return false;
    }
    if official.eq_ignore_ascii_case(installed) {
        return true;
    }
    let mut left = numeric_segments(official);
    let mut right = numeric_segments(installed);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    while left.last() == Some(&0) {
        left.pop();
    }
    while right.last() == Some(&0) {
        right.pop();
    }
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left.len() <= right.len() && left.iter().zip(right.iter()).all(|(a, b)| a == b)
}

fn numeric_segments(value: &str) -> Vec<u64> {
    let value = value.trim().trim_start_matches(['v', 'V']);
    let mut out = Vec::new();
    for part in value.split('.') {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            if out.is_empty() {
                continue;
            }
            break;
        }
        let Ok(n) = digits.parse::<u64>() else {
            break;
        };
        out.push(n);
        let rest = &part[digits.len()..];
        if !rest.trim().is_empty() {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn prefix_matches_and_other_patches_do_not() {
        assert!(version_matches("1.6", "1.6.1170"));
        assert!(version_matches("1.6.1170", "1.6.1170.0"));
        assert!(version_matches("1.6.1170.0", "1.6.1170"));
        assert!(!version_matches("1.6.1170", "1.6.640"));
        assert!(!version_matches("1.6.1170", "1.6"));
    }

    #[test]
    fn unknown_when_nothing_declares_support() {
        let advice = advise(Some("1.6.1170"), &[], &[]);
        assert_eq!(advice.status, ModVersionStatus::Unknown);
        assert_eq!(advice.message, "Could not detect supported game version");
    }

    #[test]
    fn mismatch_when_tags_and_collections_disagree_with_the_install() {
        let advice = advise(Some("1.6.1170"), &tags(&["1.6.1170"]), &tags(&["1.6.640"]));
        assert_eq!(advice.status, ModVersionStatus::Mismatch);
        assert_eq!(advice.message, "Does not officially support 1.6.1170");
    }

    #[test]
    fn tags_alone_can_match() {
        let advice = advise(Some("1.6.1170.0"), &tags(&["1.6"]), &[]);
        assert_eq!(advice.status, ModVersionStatus::Ok);
        assert!(advice.message.is_empty());
    }

    #[test]
    fn does_not_claim_a_mismatch_when_the_install_version_is_unknown() {
        let advice = advise(None, &tags(&["1.5"]), &[]);
        assert_eq!(advice.status, ModVersionStatus::Ok);
    }
}
