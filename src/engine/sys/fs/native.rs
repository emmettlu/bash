//! Native filesystem utilities for the Windows-only runtime.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::engine::error;

// Selectively re-export unsupported fallbacks that we don't override.
pub(crate) use crate::engine::sys::unsupported::fs::MetadataExt;

/// 宿主进程 PATHEXT 的缓存, 用于没有 Shell 环境上下文的调用路径.
/// Shell PATH 搜索会显式传入当前 Shell 的 PATHEXT, 不使用此缓存.
static PATHEXT_EXTENSIONS: LazyLock<Vec<String>> = LazyLock::new(|| {
    executable_extensions_from_pathext(
        &std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string()),
    )
});

/// 将 PATHEXT 值解析为规范化的小写扩展名列表.
pub fn executable_extensions_from_pathext(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                None
            } else if entry.starts_with('.') {
                Some(entry.to_ascii_lowercase())
            } else {
                Some(format!(".{entry}").to_ascii_lowercase())
            }
        })
        .collect()
}

/// 返回宿主进程 PATHEXT 对应的扩展名.
pub fn default_executable_extensions() -> &'static [String] {
    PATHEXT_EXTENSIONS.as_slice()
}

/// Returns the stem of a PATHEXT entry (with any leading `.` removed).
///
/// `PATHEXT` canonically stores entries like `.EXE`, but tolerant parsing
/// accepts entries without the leading dot too.
fn pathext_entry_stem(entry: &str) -> &str {
    entry.strip_prefix('.').unwrap_or(entry)
}

/// Returns true if the path's extension is in the PATHEXT list.
///
/// Performs case-insensitive comparison against the cached PATHEXT entries
/// without allocating.
pub fn has_executable_extension(path: &Path) -> bool {
    has_executable_extension_with_extensions(path, default_executable_extensions())
}

/// 使用指定 PATHEXT 扩展名判断路径是否可执行.
pub fn has_executable_extension_with_extensions(path: &Path, extensions: &[String]) -> bool {
    path.extension().is_some_and(|ext| {
        extensions
            .iter()
            .any(|entry| ext.eq_ignore_ascii_case(pathext_entry_stem(entry)))
    })
}

/// Returns true if `path` is, by itself, an existing executable file.
///
/// Used both for the initial check in [`resolve_executable`] and for
/// [`PathExt::executable`].
fn is_executable_file(path: &Path) -> bool {
    is_executable_file_with_extensions(path, default_executable_extensions())
}

fn is_executable_file_with_extensions(path: &Path, extensions: &[String]) -> bool {
    has_executable_extension_with_extensions(path, extensions) && path.is_file()
}

/// Resolves an owned path to the actual on-disk executable file, if any.
///
/// If the path is already a file with a `PATHEXT` extension, it is returned
/// unchanged (no allocation). Otherwise, each `PATHEXT` extension is appended
/// in turn and the first existing file is returned.
pub fn resolve_executable(path: PathBuf) -> Option<PathBuf> {
    resolve_executable_with_extensions(path, default_executable_extensions())
}

/// 使用指定 PATHEXT 扩展名解析可执行文件.
pub fn resolve_executable_with_extensions(path: PathBuf, extensions: &[String]) -> Option<PathBuf> {
    if is_executable_file_with_extensions(&path, extensions) {
        return Some(path);
    }
    for extension in extensions {
        let mut name = path.as_os_str().to_owned();
        name.push(extension);
        let candidate = PathBuf::from(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

impl crate::engine::sys::traits::PathExt for Path {
    fn readable(&self) -> bool {
        self.exists()
    }

    fn writable(&self) -> bool {
        self.metadata().is_ok_and(|m| !m.permissions().readonly())
    }

    fn executable(&self) -> bool {
        if is_executable_file(self) {
            return true;
        }
        // Try each PATHEXT extension without allocating a separate PathBuf
        // per candidate until one exists.
        PATHEXT_EXTENSIONS.iter().any(|ext| {
            let mut name = self.as_os_str().to_owned();
            name.push(ext);
            Self::new(&name).is_file()
        })
    }

    fn exists_and_is_block_device(&self) -> bool {
        false
    }

    fn exists_and_is_char_device(&self) -> bool {
        false
    }

    fn exists_and_is_fifo(&self) -> bool {
        false
    }

    fn exists_and_is_socket(&self) -> bool {
        false
    }

    fn exists_and_is_setgid(&self) -> bool {
        false
    }

    fn exists_and_is_setuid(&self) -> bool {
        false
    }

    fn exists_and_is_sticky_bit(&self) -> bool {
        false
    }

    fn get_device_and_inode(&self) -> Result<(u64, u64), crate::engine::error::Error> {
        // TODO(windows): implement using file index / volume serial number.
        Err(error::ErrorKind::NotSupported("get_device_and_inode").into())
    }
}

/// Splits a PATH-like value into individual paths.
///
/// On Windows, this delegates to [`std::env::split_paths`].
pub fn split_paths<T: AsRef<OsStr> + ?Sized>(s: &T) -> std::env::SplitPaths<'_> {
    std::env::split_paths(s)
}

/// Opens a null file that will discard all I/O.
pub fn open_null_file() -> Result<std::fs::File, error::Error> {
    let f = std::fs::File::options()
        .read(true)
        .write(true)
        .open("NUL")?;
    Ok(f)
}

/// Handles shell special file paths that do not exist as native Windows paths.
pub fn try_open_special_file(path: &Path) -> Option<Result<std::fs::File, std::io::Error>> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized == "/dev/null" {
        Some(open_null_file().map_err(std::io::Error::other))
    } else {
        None
    }
}

/// Returns the default paths where executables are typically found on Windows.
pub(crate) fn get_default_executable_search_paths() -> Vec<PathBuf> {
    default_system_paths()
}

/// Returns the default paths where standard system utilities are found on Windows.
pub fn get_default_standard_utils_paths() -> Vec<PathBuf> {
    default_system_paths()
}

fn default_system_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(sysroot) = std::env::var("SystemRoot") {
        paths.push(PathBuf::from(&sysroot).join("system32"));
        paths.push(PathBuf::from(&sysroot));
        paths.push(PathBuf::from(&sysroot).join("System32").join("Wbem"));
        paths.push(
            PathBuf::from(&sysroot)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0"),
        );
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        paths.push(
            PathBuf::from(userprofile)
                .join("AppData")
                .join("Local")
                .join("Microsoft")
                .join("WindowsApps"),
        );
    }
    paths
}

/// Returns the path to the system-wide shell profile script.
///
/// On Windows, no system profile is loaded by default.
pub const fn get_system_profile_path() -> Option<&'static Path> {
    None
}

/// Returns the path to the system-wide shell rc script.
///
/// On Windows, no system rc file is loaded by default.
pub const fn get_system_rc_path() -> Option<&'static Path> {
    None
}

/// Returns the default for case-insensitive pathname expansion.
///
/// On Windows, filesystems are typically case-insensitive, so this returns `true`.
pub const fn default_case_insensitive_path_expansion() -> bool {
    true
}

/// Path separator characters on Windows.
const PATH_SEPARATORS: [char; 2] = ['/', '\\'];

/// Returns true if the string contains a path separator character.
///
/// On Windows, both `/` and `\` are considered path separators.
pub fn contains_path_separator(s: &str) -> bool {
    s.contains(PATH_SEPARATORS)
}

/// Returns true if the string ends with a path separator character.
///
/// On Windows, both `/` and `\` are considered path separators.
pub fn ends_with_path_separator(s: &str) -> bool {
    s.ends_with(PATH_SEPARATORS)
}

/// Returns the string with a trailing path separator removed, if present.
///
/// On Windows, both `/` and `\` are considered path separators.
pub fn strip_path_separator_suffix(s: &str) -> &str {
    s.strip_suffix(PATH_SEPARATORS).unwrap_or(s)
}

/// Finds the byte index of the last path separator in the string.
///
/// On Windows, both `/` and `\` are considered path separators.
pub fn rfind_path_separator(s: &str) -> Option<usize> {
    s.rfind(PATH_SEPARATORS)
}

/// Splits a string on path separator characters, returning an iterator of components.
///
/// On Windows, both `/` and `\` are used as separators.
pub fn split_path_for_pattern(s: &str) -> impl Iterator<Item = &str> {
    s.split(PATH_SEPARATORS)
}

/// Returns the root path for an absolute pattern, if the first component indicates one.
///
/// On Windows, recognizes both a leading separator (empty first component from splitting
/// a path like `/foo`) and a drive-letter prefix like `C:` as absolute.
///
/// TODO(windows): UNC paths like `\\server\share\foo` are not yet handled
/// specially; they split into `["", "", "server", "share", "foo"]`, and the
/// leading empty component causes them to be treated as if they were rooted
/// at `/`, which drops the server/share portion. Supporting UNC requires
/// peeking further into the component list.
pub fn pattern_path_root(first_component: &str) -> Option<PathBuf> {
    if first_component.is_empty() {
        // Leading separator, e.g. `/foo` split into ["", "foo"].
        Some(PathBuf::from("/"))
    } else if first_component.len() == 2
        && first_component.as_bytes()[0].is_ascii_alphabetic()
        && first_component.as_bytes()[1] == b':'
    {
        // Drive letter prefix, e.g. `c:/foo` split into ["c:", "foo"].
        let mut root = String::with_capacity(3);
        root.push_str(first_component);
        root.push('/');
        Some(PathBuf::from(root))
    } else {
        None
    }
}

/// Pushes a component onto a path for pattern expansion.
///
/// On Windows, `PathBuf::push` has special drive-letter and root-replacement
/// semantics that conflict with shell path construction (e.g. pushing `C:foo`
/// onto `D:\bar` replaces the whole path). This function always appends the
/// component as a child, operating on the underlying `OsString` so non-UTF-8
/// content in the path is preserved and no reallocation is needed.
pub fn push_path_for_pattern(path: &mut PathBuf, component: &str) {
    // Separator characters are ASCII, and WTF-8-encoded OsStr bytes are a
    // superset of UTF-8, so checking the last byte directly is safe.
    let bytes = path.as_os_str().as_encoded_bytes();
    let needs_sep = !bytes.is_empty() && !matches!(bytes.last(), Some(b'/' | b'\\'));

    let buf = path.as_mut_os_string();
    if needs_sep {
        buf.push("/");
    }
    buf.push(component);
}

/// Normalizes path separators for shell output.
///
/// On Windows, replaces `\` with `/` since backslash is the shell escape character.
pub fn normalize_path_separators(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains('\\') {
        std::borrow::Cow::Owned(s.replace('\\', "/"))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// Converts a `Path` to a Unix-style string for shell UI output.
/// Replaces `\` with `/` so paths look like Unix.
pub fn display_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Wrapper that displays a `PathBuf` with Unix-style forward slashes on Windows.
pub struct DisplayPath(pub std::path::PathBuf);

impl std::fmt::Display for DisplayPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", display_path(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_separator_helpers_both_slashes() {
        assert!(contains_path_separator("foo/bar"));
        assert!(contains_path_separator(r"foo\bar"));
        assert!(contains_path_separator(r"mixed/and\back"));
        assert!(!contains_path_separator("foobar"));

        assert!(ends_with_path_separator("foo/"));
        assert!(ends_with_path_separator(r"foo\"));
        assert!(!ends_with_path_separator("foo"));

        assert_eq!(strip_path_separator_suffix("foo/"), "foo");
        assert_eq!(strip_path_separator_suffix(r"foo\"), "foo");
        assert_eq!(strip_path_separator_suffix("foo"), "foo");

        assert_eq!(rfind_path_separator("a/b/c"), Some(3));
        assert_eq!(rfind_path_separator(r"a\b\c"), Some(3));
        assert_eq!(rfind_path_separator(r"a/b\c"), Some(3));
        assert_eq!(rfind_path_separator("abc"), None);
    }

    #[test]
    fn split_path_for_pattern_both_slashes() {
        let parts: Vec<_> = split_path_for_pattern("a/b/c").collect();
        assert_eq!(parts, vec!["a", "b", "c"]);

        let parts: Vec<_> = split_path_for_pattern(r"a\b\c").collect();
        assert_eq!(parts, vec!["a", "b", "c"]);

        let parts: Vec<_> = split_path_for_pattern(r"a/b\c").collect();
        assert_eq!(parts, vec!["a", "b", "c"]);

        let parts: Vec<_> = split_path_for_pattern("/a/b").collect();
        assert_eq!(parts, vec!["", "a", "b"]);
    }

    #[test]
    fn pattern_path_root_leading_separator() {
        assert_eq!(pattern_path_root(""), Some(PathBuf::from("/")));
    }

    #[test]
    fn pattern_path_root_drive_letters() {
        assert_eq!(pattern_path_root("c:"), Some(PathBuf::from("c:/")));
        assert_eq!(pattern_path_root("C:"), Some(PathBuf::from("C:/")));
        assert_eq!(pattern_path_root("Z:"), Some(PathBuf::from("Z:/")));
    }

    #[test]
    fn pattern_path_root_rejects_non_drive_two_char_prefix() {
        // "1:" is not a valid drive letter — must be alphabetic.
        assert_eq!(pattern_path_root("1:"), None);
        // Longer drive-like strings are not treated as roots.
        assert_eq!(pattern_path_root("cd"), None);
        assert_eq!(pattern_path_root("c:\\"), None);
        assert_eq!(pattern_path_root("foo"), None);
    }

    #[test]
    fn push_path_for_pattern_appends_with_forward_slash() {
        let mut p = PathBuf::from(r"C:\Users\reuben");
        push_path_for_pattern(&mut p, "foo");
        // Forward slash is used as the appended separator, yielding mixed
        // separators — acceptable because `normalize_path_separators` is
        // applied downstream before display.
        assert_eq!(p, PathBuf::from(r"C:\Users\reuben/foo"));
    }

    #[test]
    fn push_path_for_pattern_no_double_separator() {
        let mut p = PathBuf::from("C:/Users/reuben/");
        push_path_for_pattern(&mut p, "foo");
        assert_eq!(p, PathBuf::from("C:/Users/reuben/foo"));

        let mut p = PathBuf::from(r"C:\Users\reuben\");
        push_path_for_pattern(&mut p, "foo");
        assert_eq!(p, PathBuf::from(r"C:\Users\reuben\foo"));
    }

    #[test]
    fn push_path_for_pattern_onto_drive_root() {
        let mut p = PathBuf::from("c:/");
        push_path_for_pattern(&mut p, "foo");
        assert_eq!(p, PathBuf::from("c:/foo"));
    }

    #[test]
    fn push_path_for_pattern_onto_empty() {
        let mut p = PathBuf::new();
        push_path_for_pattern(&mut p, "foo");
        // Empty path stays un-prefixed — we only add a separator between
        // existing content and the new component.
        assert_eq!(p, PathBuf::from("foo"));
    }

    #[test]
    fn normalize_path_separators_converts_backslashes() {
        use std::borrow::Cow;
        // Already-forward-slashed input is borrowed (no allocation).
        assert!(matches!(
            normalize_path_separators("c:/foo/bar"),
            Cow::Borrowed("c:/foo/bar")
        ));
        // Mixed or backslashed input becomes owned and fully forward-slashed.
        let normalized = normalize_path_separators(r"c:\foo\bar");
        assert_eq!(normalized.as_ref(), "c:/foo/bar");
        let normalized = normalize_path_separators(r"c:\foo/bar");
        assert_eq!(normalized.as_ref(), "c:/foo/bar");
    }

    #[test]
    fn default_case_insensitive_is_true() {
        assert!(default_case_insensitive_path_expansion());
    }

    #[test]
    fn has_executable_extension_is_case_insensitive() {
        // Force the PATHEXT cache for this test's defaults.
        assert!(has_executable_extension(Path::new("foo.exe")));
        assert!(has_executable_extension(Path::new("foo.EXE")));
        assert!(has_executable_extension(Path::new("foo.Cmd")));
        assert!(!has_executable_extension(Path::new("foo.txt")));
        assert!(!has_executable_extension(Path::new("foo")));
    }

    #[test]
    fn pathext_entry_stem_strips_dot() {
        assert_eq!(pathext_entry_stem(".exe"), "exe");
        assert_eq!(pathext_entry_stem(".cmd"), "cmd");
        // Tolerant: entries without a leading dot are returned as-is.
        assert_eq!(pathext_entry_stem("exe"), "exe");
        assert_eq!(pathext_entry_stem(""), "");
    }

    #[test]
    fn pathext_parser_normalizes_entries() {
        assert_eq!(
            executable_extensions_from_pathext(".EXE;CMD;; .Bat "),
            vec![".exe", ".cmd", ".bat"]
        );
    }

    #[test]
    fn special_file_matching_is_exact() {
        assert!(try_open_special_file(Path::new("/dev/null")).is_some());
        assert!(try_open_special_file(Path::new(r"\dev\null")).is_some());
        assert!(try_open_special_file(Path::new("relative/dev/null")).is_none());
        assert!(try_open_special_file(Path::new("C:/dev/null")).is_none());
    }

    #[test]
    fn resolve_executable_for_nonexistent_returns_none() {
        // A path that cannot exist on any test host.
        let path = PathBuf::from(r"C:\\__bash_test_definitely_missing__");
        assert!(resolve_executable(path).is_none());
    }
}
