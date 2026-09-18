//! Per-mod options declared by the mod itself.
//!
//! Helldivers 2 mods built for Arsenal / HD2MM ship a `manifest.json` that
//! splits the package into togglable options and mutually exclusive
//! sub-options. A plugin turns that into a [`ModOptionSet`]; the user's picks
//! are stored as a [`ModOptionSelection`] on the staged mod, and deploy runs
//! every staged path through an [`IncludeFilter`] built from the two.
//!
//! An include folder is a *container*: its contents deploy as if they sat at
//! the mod root, which is why `Body/Blue/x.patch_0` lands in `data/` and not
//! `data/Body/Blue/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::games::normalize_relative;

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModOptionSet {
    pub description: Option<String>,
    /// Absolute path to the mod icon, for the asset protocol.
    pub icon: Option<String>,
    pub options: Vec<ModOption>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModOption {
    /// Stable across manifest edits that only reorder or retitle entries.
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// Absolute path to the preview image, for the asset protocol.
    pub image: Option<String>,
    /// Root-relative folders this option contributes when selected.
    pub include: Vec<String>,
    pub sub_options: Vec<ModSubOption>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ModSubOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModOptionSelection {
    #[serde(default)]
    pub enabled_options: Vec<String>,
    /// Option id -> chosen sub-option id.
    #[serde(default)]
    pub sub_choice: BTreeMap<String, String>,
}

impl ModOptionSet {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }
}

/// Slug used as an option id. Includes the index so two options sharing a name
/// stay distinguishable, and survives a retitle better than the raw name.
pub fn option_id(index: usize, name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("opt-{index}")
    } else {
        format!("{index}-{slug}")
    }
}

pub fn sub_option_id(parent_id: &str, index: usize, name: &str) -> String {
    format!("{parent_id}.{}", option_id(index, name))
}

/// Everything on, first sub-option picked. An Arsenal mod whose content lives
/// entirely inside options would otherwise deploy nothing on first install.
pub fn default_selection(set: &ModOptionSet) -> ModOptionSelection {
    let mut selection = ModOptionSelection::default();
    for option in &set.options {
        selection.enabled_options.push(option.id.clone());
        if let Some(first) = option.sub_options.first() {
            selection
                .sub_choice
                .insert(option.id.clone(), first.id.clone());
        }
    }
    selection
}

/// Drop ids the manifest no longer knows, and make sure every enabled option
/// with sub-options has exactly one valid choice.
pub fn normalize_selection(
    set: &ModOptionSet,
    selection: &ModOptionSelection,
) -> ModOptionSelection {
    let mut out = ModOptionSelection::default();
    for option in &set.options {
        if !selection.enabled_options.contains(&option.id) {
            continue;
        }
        out.enabled_options.push(option.id.clone());
        if option.sub_options.is_empty() {
            continue;
        }
        let chosen = selection
            .sub_choice
            .get(&option.id)
            .filter(|id| option.sub_options.iter().any(|s| &&s.id == id))
            .cloned()
            .unwrap_or_else(|| option.sub_options[0].id.clone());
        out.sub_choice.insert(option.id.clone(), chosen);
    }
    out
}

/// The selection to act on: the stored one (normalized) or the default.
pub fn effective_selection(
    set: &ModOptionSet,
    stored: Option<&ModOptionSelection>,
) -> ModOptionSelection {
    match stored {
        Some(selection) => normalize_selection(set, selection),
        None => default_selection(set),
    }
}

/// Decides which staged paths deploy, and where they land relative to the mod
/// root once their include folder is peeled off.
#[derive(Debug, Clone, Default)]
pub struct IncludeFilter {
    has_options: bool,
    /// Every include path in the manifest, selected or not.
    managed: Vec<PathBuf>,
    /// Include paths contributed by the current selection.
    active: Vec<PathBuf>,
}

impl IncludeFilter {
    pub fn build(set: &ModOptionSet, stored: Option<&ModOptionSelection>) -> Self {
        if set.is_empty() {
            return Self::default();
        }
        let selection = effective_selection(set, stored);
        let mut managed = Vec::new();
        let mut active = Vec::new();

        for option in &set.options {
            let enabled = selection.enabled_options.contains(&option.id);
            for path in &option.include {
                push_include(&mut managed, path);
                if enabled {
                    push_include(&mut active, path);
                }
            }
            let chosen = selection.sub_choice.get(&option.id);
            for sub in &option.sub_options {
                for path in &sub.include {
                    push_include(&mut managed, path);
                    if enabled && chosen == Some(&sub.id) {
                        push_include(&mut active, path);
                    }
                }
            }
        }

        Self {
            has_options: true,
            managed,
            active,
        }
    }

    /// True when the filter has nothing to say and every staged path deploys.
    pub fn is_passthrough(&self) -> bool {
        !self.has_options
    }

    /// `None` means the path must not deploy. `Some(path)` is where it goes,
    /// relative to the mod root, with its include folder stripped.
    pub fn map(&self, relative: &Path, is_dir: bool) -> Option<PathBuf> {
        if !self.has_options {
            return Some(relative.to_path_buf());
        }
        let rel = normalize_relative(relative);

        if let Some(include) = longest_prefix(&self.active, &rel) {
            let stripped = rel.strip_prefix(include).ok()?;
            if stripped.as_os_str().is_empty() {
                // The include folder itself is scaffolding; its files carry the
                // content and create whatever parents they need.
                return None;
            }
            return Some(stripped.to_path_buf());
        }

        // Inside an unselected include, or a folder on the way to one.
        if self
            .managed
            .iter()
            .any(|m| rel.starts_with(m) || m.starts_with(&rel))
        {
            return None;
        }

        // Unmapped content. Root files ship with the mod; folders the manifest
        // never mentions are packaging leftovers that Arsenal ignores too.
        if !is_dir && rel.components().count() == 1 {
            Some(rel)
        } else {
            None
        }
    }
}

fn push_include(list: &mut Vec<PathBuf>, raw: &str) {
    let path = normalize_relative(Path::new(raw));
    if path.as_os_str().is_empty() || list.contains(&path) {
        return;
    }
    list.push(path);
}

fn longest_prefix<'a>(candidates: &'a [PathBuf], rel: &Path) -> Option<&'a PathBuf> {
    candidates
        .iter()
        .filter(|c| rel.starts_with(c))
        .max_by_key(|c| c.components().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_with_options() -> ModOptionSet {
        ModOptionSet {
            description: None,
            icon: None,
            options: vec![
                ModOption {
                    id: "0-head".into(),
                    name: "Head".into(),
                    description: None,
                    image: None,
                    include: vec!["Head".into()],
                    sub_options: Vec::new(),
                },
                ModOption {
                    id: "1-body".into(),
                    name: "Body".into(),
                    description: None,
                    image: None,
                    include: Vec::new(),
                    sub_options: vec![
                        ModSubOption {
                            id: "1-body.0-blue".into(),
                            name: "Blue".into(),
                            description: None,
                            image: None,
                            include: vec!["Body/Blue".into()],
                        },
                        ModSubOption {
                            id: "1-body.1-red".into(),
                            name: "Red".into(),
                            description: None,
                            image: None,
                            include: vec!["Body/Red".into()],
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn default_selection_enables_all_and_picks_first_sub() {
        let set = set_with_options();
        let selection = default_selection(&set);
        assert_eq!(selection.enabled_options, vec!["0-head", "1-body"]);
        assert_eq!(selection.sub_choice.get("1-body").unwrap(), "1-body.0-blue");
        assert!(!selection.sub_choice.contains_key("0-head"));
    }

    #[test]
    fn empty_option_set_is_passthrough() {
        let filter = IncludeFilter::build(&ModOptionSet::default(), None);
        assert!(filter.is_passthrough());
        assert_eq!(
            filter.map(Path::new("anything/deep/file.bin"), false),
            Some(PathBuf::from("anything/deep/file.bin"))
        );
    }

    #[test]
    fn include_folder_is_peeled_off_the_deploy_path() {
        let set = set_with_options();
        let filter = IncludeFilter::build(&set, None);
        assert_eq!(
            filter.map(Path::new("Head/abc.patch_0"), false),
            Some(PathBuf::from("abc.patch_0"))
        );
        assert_eq!(
            filter.map(Path::new("Body/Blue/abc.patch_0"), false),
            Some(PathBuf::from("abc.patch_0"))
        );
    }

    #[test]
    fn unselected_option_and_sub_option_are_excluded() {
        let set = set_with_options();
        let selection = ModOptionSelection {
            enabled_options: vec!["1-body".into()],
            sub_choice: [("1-body".to_string(), "1-body.1-red".to_string())]
                .into_iter()
                .collect(),
        };
        let filter = IncludeFilter::build(&set, Some(&selection));
        assert_eq!(filter.map(Path::new("Head/abc.patch_0"), false), None);
        assert_eq!(filter.map(Path::new("Body/Blue/abc.patch_0"), false), None);
        assert_eq!(
            filter.map(Path::new("Body/Red/abc.patch_0"), false),
            Some(PathBuf::from("abc.patch_0"))
        );
    }

    #[test]
    fn root_files_deploy_but_unmapped_folders_do_not() {
        let set = set_with_options();
        let filter = IncludeFilter::build(&set, None);
        assert_eq!(
            filter.map(Path::new("loose.patch_0"), false),
            Some(PathBuf::from("loose.patch_0"))
        );
        assert_eq!(filter.map(Path::new("Unused/abc.patch_0"), false), None);
        assert_eq!(filter.map(Path::new("Unused"), true), None);
    }

    #[test]
    fn include_scaffold_dirs_are_never_deployed() {
        let set = set_with_options();
        let filter = IncludeFilter::build(&set, None);
        assert_eq!(filter.map(Path::new("Head"), true), None);
        assert_eq!(filter.map(Path::new("Body"), true), None);
        assert_eq!(filter.map(Path::new("Body/Blue"), true), None);
    }

    #[test]
    fn normalize_drops_unknown_ids_and_repairs_sub_choice() {
        let set = set_with_options();
        let selection = ModOptionSelection {
            enabled_options: vec!["1-body".into(), "9-gone".into()],
            sub_choice: [("1-body".to_string(), "1-body.7-missing".to_string())]
                .into_iter()
                .collect(),
        };
        let out = normalize_selection(&set, &selection);
        assert_eq!(out.enabled_options, vec!["1-body"]);
        assert_eq!(out.sub_choice.get("1-body").unwrap(), "1-body.0-blue");
    }

    #[test]
    fn windows_separators_in_include_paths_still_match() {
        let set = ModOptionSet {
            options: vec![ModOption {
                id: "0-armor".into(),
                name: "Armor".into(),
                include: vec!["Armor 2\\Helmet A".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let filter = IncludeFilter::build(&set, None);
        assert_eq!(
            filter.map(Path::new("Armor 2/Helmet A/x.patch_0"), false),
            Some(PathBuf::from("x.patch_0"))
        );
    }
}
