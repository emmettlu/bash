//! Path searching utilities.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use crate::engine::sys;

/// Encapsulates the result of a path search.
pub struct ExecutablePathSearch<PI, N> {
    paths: VecDeque<PI>,
    filename: N,
    executable_extensions: Vec<String>,
}

impl<PI, N> Iterator for ExecutablePathSearch<PI, N>
where
    PI: AsRef<Path>,
    N: AsRef<Path>,
{
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(path) = self.paths.pop_front() {
            let path = PathBuf::from(path.as_ref()).join(self.filename.as_ref());
            // Skip directories outright, then resolve the path to an actual
            // executable file, including PATHEXT probing when needed.
            if path.is_dir() {
                continue;
            }
            if let Some(resolved) =
                sys::fs::resolve_executable_with_extensions(path, &self.executable_extensions)
            {
                return Some(resolved);
            }
        }
        None
    }
}

pub(crate) struct ExecutablePathPrefixSearch<P> {
    paths: P,
    queued_items: VecDeque<PathBuf>,
    filename_prefix: String,
    case_insensitive: bool,
    executable_extensions: Vec<String>,
}

impl<P> Iterator for ExecutablePathPrefixSearch<P>
where
    P: Iterator,
    P::Item: AsRef<Path>,
{
    type Item = PathBuf;

    fn next(&mut self) -> Option<Self::Item> {
        // If we already found some items and queued them, then yield one now.
        if let Some(item) = self.queued_items.pop_front() {
            return Some(item);
        }

        for path in self.paths.by_ref() {
            let path = PathBuf::from(path.as_ref());

            if let Ok(readdir) = path.read_dir() {
                for entry in readdir.flatten() {
                    if let Ok(mut filename) = entry.file_name().into_string() {
                        if self.case_insensitive {
                            filename = filename.to_ascii_lowercase();
                        }

                        if !filename.starts_with(&self.filename_prefix) {
                            continue;
                        }
                    }

                    let entry_path = entry.path();
                    if let Ok(file_type) = entry.file_type()
                        && (file_type.is_file() || file_type.is_symlink())
                        && entry_path.is_file()
                        && sys::fs::has_executable_extension_with_extensions(
                            entry_path.as_path(),
                            &self.executable_extensions,
                        )
                    {
                        self.queued_items.push_back(entry_path);
                    }
                }
            }
            if let Some(item) = self.queued_items.pop_front() {
                return Some(item);
            }
        }

        None
    }
}

/// Search for the given executable name in the provided paths.
///
/// # Arguments
///
/// * `paths` - An iterator over the paths to search.
/// * `filename` - The name of the executable file to search for.
pub fn search_for_executable<P, PI, N>(paths: P, filename: N) -> ExecutablePathSearch<PI, N>
where
    P: Iterator<Item = PI>,
    PI: AsRef<Path>,
    N: AsRef<Path>,
{
    search_for_executable_with_extensions(
        paths,
        filename,
        sys::fs::default_executable_extensions().to_vec(),
    )
}

/// 使用指定 PATHEXT 扩展名搜索可执行文件.
pub fn search_for_executable_with_extensions<P, PI, N>(
    paths: P,
    filename: N,
    executable_extensions: Vec<String>,
) -> ExecutablePathSearch<PI, N>
where
    P: Iterator<Item = PI>,
    PI: AsRef<Path>,
    N: AsRef<Path>,
{
    ExecutablePathSearch {
        paths: paths.collect(),
        filename,
        executable_extensions,
    }
}

pub(crate) fn search_for_executable_with_prefix_and_extensions<P>(
    paths: P,
    filename_prefix: &str,
    case_insensitive: bool,
    executable_extensions: Vec<String>,
) -> ExecutablePathPrefixSearch<P>
where
    P: Iterator,
    P::Item: AsRef<Path>,
{
    let stored_prefix = if case_insensitive {
        filename_prefix.to_ascii_lowercase()
    } else {
        filename_prefix.into()
    };

    ExecutablePathPrefixSearch {
        paths,
        queued_items: VecDeque::new(),
        filename_prefix: stored_prefix,
        case_insensitive,
        executable_extensions,
    }
}
