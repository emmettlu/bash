//! Path cache

use crate::engine::{error, variables};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Clone, Eq, PartialEq)]
struct ExecutableNameCacheKey {
    path_value: String,
    path_ext_value: String,
    case_insensitive: bool,
}

/// Cache of executable names for interactive command completion.
#[derive(Clone, Default)]
pub struct ExecutableNameCache {
    key: Option<ExecutableNameCacheKey>,
    names: Vec<String>,
    updated_at: Option<Instant>,
}

impl ExecutableNameCache {
    const TTL: Duration = Duration::from_secs(5);

    pub fn get_or_update(
        &mut self,
        path_value: String,
        path_ext_value: String,
        case_insensitive: bool,
        build: impl FnOnce(&str, &str, bool) -> Vec<String>,
    ) -> &[String] {
        let key = ExecutableNameCacheKey {
            path_value,
            path_ext_value,
            case_insensitive,
        };
        let expired = self
            .updated_at
            .is_none_or(|updated_at| updated_at.elapsed() >= Self::TTL);

        if self.key.as_ref() != Some(&key) || expired {
            self.names = build(&key.path_value, &key.path_ext_value, key.case_insensitive);
            self.key = Some(key);
            self.updated_at = Some(Instant::now());
        }

        &self.names
    }

    /// 清除 executable completion cache.
    pub fn reset(&mut self) {
        self.key = None;
        self.names.clear();
        self.updated_at = None;
    }
}

#[derive(Clone, Eq, PartialEq)]
struct PathCacheKey {
    path_value: String,
    path_ext_value: String,
}

/// A cache of paths associated with names.
#[derive(Clone, Default)]
pub struct PathCache {
    /// The cache itself.
    cache: std::collections::HashMap<String, PathBuf>,
    key: Option<PathCacheKey>,
}

impl PathCache {
    /// Clears all elements from the cache.
    pub fn reset(&mut self) {
        self.cache.clear();
        self.key = None;
    }

    /// PATH 或 PATHEXT 变化时清除已有命令位置缓存.
    pub fn synchronize_path_values(&mut self, path_value: &str, path_ext_value: &str) {
        let key = PathCacheKey {
            path_value: path_value.to_owned(),
            path_ext_value: path_ext_value.to_owned(),
        };
        if self.key.as_ref().is_some_and(|previous| previous != &key) {
            self.cache.clear();
        }
        self.key = Some(key);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executable_cache_resets_and_expires() {
        let mut cache = ExecutableNameCache::default();
        let names = cache.get_or_update("path".into(), ".EXE".into(), true, |_, _, _| {
            vec!["first.exe".into()]
        });
        assert_eq!(names, ["first.exe"]);

        cache.updated_at = Some(Instant::now() - ExecutableNameCache::TTL);
        let names = cache.get_or_update("path".into(), ".EXE".into(), true, |_, _, _| {
            vec!["second.exe".into()]
        });
        assert_eq!(names, ["second.exe"]);

        cache.reset();
        assert!(cache.key.is_none());
        assert!(cache.names.is_empty());
    }

    #[test]
    fn path_or_pathext_change_invalidates_location_cache() {
        let mut cache = PathCache::default();
        cache.synchronize_path_values("first", ".EXE");
        cache.set("command", PathBuf::from("first/command.exe"));

        cache.synchronize_path_values("second", ".EXE");
        assert!(cache.get("command").is_none());

        cache.set("command", PathBuf::from("second/command.exe"));
        cache.synchronize_path_values("second", ".CMD");
        assert!(cache.get("command").is_none());
    }

    #[test]
    fn unchanged_path_key_preserves_location_cache() {
        let mut cache = PathCache::default();
        cache.synchronize_path_values("path", ".EXE");
        cache.set("command", PathBuf::from("path/command.exe"));

        cache.synchronize_path_values("path", ".EXE");
        assert_eq!(
            cache.get("command"),
            Some(PathBuf::from("path/command.exe"))
        );
    }
}
