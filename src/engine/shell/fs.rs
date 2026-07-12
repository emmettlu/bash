//! Filesystem interaction in the shell.

use std::path::{Component, Path, PathBuf};

use crate::engine::{
    ExecutionParameters, ShellFd,
    env::{EnvironmentLookup, EnvironmentScope},
    error, openfiles, pathsearch,
    sys::users,
    variables,
};

fn clean_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match normalized.components().next_back() {
                Some(Component::Normal(_)) => {
                    normalized.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::CurDir | Component::ParentDir) | None => {
                    normalized.push(component.as_os_str());
                }
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }

    normalized
}

impl crate::engine::Shell {
    /// Sets the shell's current working directory to the given path.
    ///
    /// # Arguments
    ///
    /// * `target_dir` - The path to set as the working directory.
    pub fn set_working_dir(&mut self, target_dir: impl AsRef<Path>) -> Result<(), error::Error> {
        let abs_path = self.absolute_path(target_dir.as_ref());

        match std::fs::metadata(&abs_path) {
            Ok(m) => {
                if !m.is_dir() {
                    return Err(error::ErrorKind::NotADirectory(abs_path).into());
                }
            }
            Err(e) => {
                return Err(e.into());
            }
        }

        // Normalize the path (but don't canonicalize it).
        let cleaned_path = clean_path_lexically(&abs_path);

        let pwd = cleaned_path.to_string_lossy().to_string();

        self.env.update_or_add(
            "PWD",
            variables::ShellValueLiteral::Scalar(pwd),
            |_| Ok(()),
            EnvironmentLookup::Anywhere,
            EnvironmentScope::Global,
        )?;
        let oldpwd = std::mem::replace(self.working_dir_mut(), cleaned_path);

        self.env.update_or_add(
            "OLDPWD",
            variables::ShellValueLiteral::Scalar(oldpwd.to_string_lossy().to_string()),
            |_| Ok(()),
            EnvironmentLookup::Anywhere,
            EnvironmentScope::Global,
        )?;

        // 相对 PATH 项会随工作目录改变, 因此两类路径缓存都必须失效.
        self.reset_path_caches();
        Ok(())
    }

    /// Tilde-shortens the given string, replacing the user's home directory with a tilde.
    ///
    /// # Arguments
    ///
    /// * `s` - The string to shorten.
    pub fn tilde_shorten(&self, s: String) -> String {
        if let Some(home_dir) = self.home_dir()
            && let Some(stripped) = s.strip_prefix(home_dir.to_string_lossy().as_ref())
        {
            return format!("~{stripped}");
        }
        s
    }

    /// Returns the shell's current home directory, if available.
    pub(crate) fn home_dir(&self) -> Option<PathBuf> {
        if let Some(home) = self.env.get_str("HOME", self) {
            Some(PathBuf::from(home.to_string()))
        } else {
            // HOME isn't set, so let's sort it out ourselves.
            users::get_current_user_home_dir()
        }
    }

    /// Finds executables in the shell's current default PATH, matching the given glob pattern.
    ///
    /// # Arguments
    ///
    /// * `required_glob_pattern` - The glob pattern to match against.
    pub fn find_executables_in_path<'a>(
        &'a self,
        filename: &'a str,
    ) -> impl Iterator<Item = PathBuf> + 'a {
        let path_var = self.env.get_str("PATH", self).unwrap_or_default();
        let paths = crate::engine::sys::fs::split_paths(path_var.as_ref());

        pathsearch::search_for_executable_with_extensions(
            paths,
            filename,
            self.executable_extensions(),
        )
    }

    /// Finds executables in the shell's current default PATH, with filenames matching the
    /// given prefix.
    ///
    /// # Arguments
    ///
    /// * `filename_prefix` - The prefix to match against executable filenames.
    pub fn find_executables_in_path_with_prefix(
        &self,
        filename_prefix: &str,
        case_insensitive: bool,
    ) -> impl Iterator<Item = PathBuf> {
        let path_var = self.env.get_str("PATH", self).unwrap_or_default();
        let paths = crate::engine::sys::fs::split_paths(path_var.as_ref()).collect::<Vec<_>>();

        pathsearch::search_for_executable_with_prefix_and_extensions(
            paths.into_iter(),
            filename_prefix,
            case_insensitive,
            self.executable_extensions(),
        )
    }

    /// Finds executable names in PATH with the given prefix, reusing a cache while PATH is stable.
    pub fn find_executable_names_in_path_with_prefix_using_cache(
        &mut self,
        filename_prefix: &str,
        case_insensitive: bool,
    ) -> Vec<String> {
        let path_value = self.env_str("PATH").unwrap_or_default().into_owned();
        let path_ext_value = self
            .env_str("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
            .into_owned();
        let cached_names = self.external_command_completion_cache.get_or_update(
            path_value,
            path_ext_value,
            case_insensitive,
            |path_value, path_ext_value, case_insensitive| {
                let paths = crate::engine::sys::fs::split_paths(path_value);
                let executable_extensions =
                    crate::engine::sys::fs::executable_extensions_from_pathext(path_ext_value);
                let mut names = pathsearch::search_for_executable_with_prefix_and_extensions(
                    paths,
                    "",
                    case_insensitive,
                    executable_extensions,
                )
                .filter_map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                })
                .collect::<Vec<_>>();
                names.sort();
                names.dedup();
                names
            },
        );

        if case_insensitive {
            let prefix = filename_prefix.to_ascii_lowercase();
            cached_names
                .iter()
                .filter(|name| name.to_ascii_lowercase().starts_with(&prefix))
                .cloned()
                .collect()
        } else {
            cached_names
                .iter()
                .filter(|name| name.starts_with(filename_prefix))
                .cloned()
                .collect()
        }
    }

    /// Determines whether the given filename is the name of an executable in one of the
    /// directories in the shell's current PATH. If found, returns the path.
    ///
    /// # Arguments
    ///
    /// * `candidate_name` - The name of the file to look for.
    pub fn find_first_executable_in_path<S: AsRef<str>>(
        &self,
        candidate_name: S,
    ) -> Option<PathBuf> {
        self.find_executables_in_path(candidate_name.as_ref())
            .next()
    }

    /// Uses the shell's hash-based path cache to check whether the given filename is the name
    /// of an executable in one of the directories in the shell's current PATH. If found,
    /// ensures the path is in the cache and returns it.
    ///
    /// # Arguments
    ///
    /// * `candidate_name` - The name of the file to look for.
    pub fn find_first_executable_in_path_using_cache<S: AsRef<str>>(
        &mut self,
        candidate_name: S,
    ) -> Option<PathBuf>
    where
        String: From<S>,
    {
        let path_value = self.env_str("PATH").unwrap_or_default().into_owned();
        let path_ext_value = self
            .env_str("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
            .into_owned();
        self.program_location_cache_mut()
            .synchronize_path_values(&path_value, &path_ext_value);

        if let Some(cached_path) = self.program_location_cache().get(&candidate_name) {
            Some(cached_path)
        } else if let Some(found_path) = self.find_first_executable_in_path(&candidate_name) {
            self.program_location_cache_mut()
                .set(candidate_name, found_path.clone());
            Some(found_path)
        } else {
            None
        }
    }

    /// 清除命令位置和 executable completion cache.
    pub fn reset_path_caches(&mut self) {
        self.program_location_cache_mut().reset();
        self.external_command_completion_cache.reset();
    }

    fn executable_extensions(&self) -> Vec<String> {
        let path_ext = self
            .env_str("PATHEXT")
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        crate::engine::sys::fs::executable_extensions_from_pathext(path_ext.as_ref())
    }

    /// Gets the absolute form of the given path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to get the absolute form of.
    pub fn absolute_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.as_os_str().is_empty() || path.is_absolute() {
            path.to_owned()
        } else {
            self.working_dir().join(path)
        }
    }

    /// Opens the given file, using the context of this shell and the provided execution parameters.
    ///
    /// # Arguments
    ///
    /// * `options` - The options to use opening the file.
    /// * `path` - The path to the file to open; may be relative to the shell's working directory.
    /// * `params` - Execution parameters.
    pub(crate) fn open_file(
        &self,
        options: &std::fs::OpenOptions,
        path: impl AsRef<Path>,
        params: &ExecutionParameters,
    ) -> Result<openfiles::OpenFile, std::io::Error> {
        // Handle shell special files before absolute_path so paths like
        // /dev/null can map to native Windows devices such as NUL.
        if let Some(result) = crate::engine::sys::fs::try_open_special_file(path.as_ref()) {
            return result.map(openfiles::OpenFile::from);
        }

        let path_to_open = self.absolute_path(path.as_ref());

        // See if this is a reference to a file descriptor, in which case the actual
        // /dev/fd* file path for this process may not match with what's in the execution
        // parameters.
        if let Some(parent) = path_to_open.parent()
            && parent == Path::new("/dev/fd")
            && let Some(filename) = path_to_open.file_name()
            && let Ok(fd_num) = filename.to_string_lossy().to_string().parse::<ShellFd>()
            && let Some(open_file) = params.fd_overlay(self).try_fd(fd_num)
        {
            return open_file.try_clone();
        }

        Ok(options.open(path_to_open)?.into())
    }

    /// Replaces the shell's currently configured open files with the given set.
    /// Typically only used by exec-like builtins.
    ///
    /// # Arguments
    ///
    /// * `open_files` - The new set of open files to use.
    pub fn replace_open_files(
        &mut self,
        open_fds: impl Iterator<Item = (ShellFd, openfiles::OpenFile)>,
    ) {
        self.open_files = openfiles::OpenFiles::from(open_fds);
    }

    pub(crate) const fn persistent_open_files(&self) -> &openfiles::OpenFiles {
        &self.open_files
    }
}
