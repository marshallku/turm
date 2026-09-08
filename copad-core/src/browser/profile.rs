//! Browser profiles — the identity a set of tabs shares its cookies with.
//!
//! One rule dominates this file and it is a migration hazard, not an aesthetic
//! preference (codex plan r1-C5): **the `default` profile must map onto the
//! platform's DEFAULT website data store**, not onto a store named "default".
//! On macOS `WKWebsiteDataStore(forIdentifier:)` creates a store that is
//! *distinct* from `WKWebsiteDataStore.default()`; adopting a named store for
//! existing users would silently sign them out of every site they are logged
//! into, with no migration path back. Named stores are therefore only ever used
//! for additional profiles the user explicitly created, and such a profile
//! starts logged out by design.

use serde::{Deserialize, Serialize};

/// The profile every pane uses unless told otherwise. Maps to the platform's
/// default (pre-existing) data store — see the module note.
pub const DEFAULT_PROFILE: &str = "default";

/// Upper bound on a profile id, so an id can be used as a single path segment
/// without truncation surprises.
const MAX_PROFILE_ID: usize = 64;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfile {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// False for a throwaway profile whose storage is dropped on close.
    #[serde(default = "default_true")]
    pub persistent: bool,
}

fn default_true() -> bool {
    true
}

impl BrowserProfile {
    pub fn default_profile() -> Self {
        Self {
            id: DEFAULT_PROFILE.to_string(),
            label: None,
            persistent: true,
        }
    }

    /// True when this profile must be backed by the platform's pre-existing
    /// default store rather than a freshly identified one.
    pub fn is_platform_default(&self) -> bool {
        self.id == DEFAULT_PROFILE
    }
}

/// A profile id becomes a directory name on Linux, so it is held to the same
/// strict charset a tab id is: no separators, no dots, no traversal.
pub fn is_valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_PROFILE_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Storage directory for a NON-default profile. Returns `None` for an invalid
/// id and for `default`, whose storage the platform owns (macOS: the default
/// `WKWebsiteDataStore`; Linux: the shared `NetworkSession` data directory).
pub fn profile_data_dir(id: &str) -> Option<std::path::PathBuf> {
    if !is_valid_profile_id(id) || id == DEFAULT_PROFILE {
        return None;
    }
    Some(
        crate::paths::state_dir()
            .join("browser")
            .join("profiles")
            .join(id),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_flagged_as_the_platform_default_store() {
        // The r1-C5 guard: whatever else changes, `default` must never be
        // handed to an "identified store" constructor.
        assert!(BrowserProfile::default_profile().is_platform_default());
        assert!(profile_data_dir(DEFAULT_PROFILE).is_none());
    }

    #[test]
    fn profile_ids_reject_path_separators_and_traversal() {
        assert!(is_valid_profile_id("work"));
        assert!(is_valid_profile_id("client-a_2"));
        assert!(!is_valid_profile_id(""));
        assert!(!is_valid_profile_id(".."));
        assert!(!is_valid_profile_id("a/b"));
        assert!(!is_valid_profile_id("a.b"));
        assert!(!is_valid_profile_id(&"x".repeat(MAX_PROFILE_ID + 1)));
    }

    #[test]
    fn a_named_profile_gets_a_confined_directory() {
        let dir = profile_data_dir("work").expect("valid id");
        assert!(dir.ends_with("browser/profiles/work"));
        assert!(profile_data_dir("../escape").is_none());
    }
}
