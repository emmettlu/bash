//! Path cache

use crate::core::{error, variables};
use std::path::PathBuf;

#[derive(Clone, Eq, PartialEq)]
struct ExecutableNameCacheKey {
    path_value: String,
    case_insensitive: bool,
}

/// Cache of executable names for interactive command completion.
#[derive(Clone, Default)]
pub struct ExecutableNameCache {
    key: Option<ExecutableNameCacheKey>,
    names: Vec<String>,
}

impl ExecutableNameCache {
    pub fn get_or_update(
        &mut self,
        path_value: String,
        case_insensitive: bool,
        build: impl FnOnce(&str, bool) -> Vec<String>,
    ) -> &[String] {
        let key = ExecutableNameCacheKey {
            path_value,
            case_insensitive,
        };

        if self.key.as_ref() != Some(&key) {
            self.names = build(&key.path_value, key.case_insensitive);
            self.key = Some(key);
        }

        &self.names
    }
}

/// A cache of paths associated with names.
#[derive(Clone, Default)]
pub struct PathCache {
    /// The cache itself.
    cache: std::collections::HashMap<String, PathBuf>,
}

impl PathCache {
    /// Clears all elements from the cache.
    pub fn reset(&mut self) {
        self.cache.clear();
    }

    /// Returns the path associated with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name to lookup.
    pub fn get<S: AsRef<str>>(&self, name: S) -> Option<PathBuf> {
        self.cache.get(name.as_ref()).cloned()
    }

    /// Sets the path associated with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - The name to set.
    /// * `path` - The path to associate with the name.
    pub fn set<T: Into<String>>(&mut self, name: T, path: PathBuf) {
        self.cache.insert(name.into(), path);
    }

    /// Projects the cache into a shell value.
    pub fn to_value(&self) -> Result<variables::ShellValue, error::Error> {
        let pairs = self
            .cache
            .iter()
            .map(|(k, v)| (Some(k.to_owned()), v.to_string_lossy().to_string()))
            .collect::<Vec<_>>();

        variables::ShellValue::associative_array_from_literals(variables::ArrayLiteral(pairs))
    }

    /// Removes the path associated with the given name, if there is one.
    /// Returns whether or not an entry was removed.
    ///
    /// # Arguments
    ///
    /// * `name` - The name to remove.
    pub fn unset<S: AsRef<str>>(&mut self, name: S) -> bool {
        self.cache.remove(name.as_ref()).is_some()
    }
}
