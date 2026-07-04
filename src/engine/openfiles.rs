//! Managing files open within a shell instance.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::process::Stdio;

use crate::engine::ShellFd;
use crate::engine::error;
use crate::engine::sys;

/// A trait representing a stream that can be read from and written to.
/// This is used for custom stream implementations in `OpenFile`.
///
/// Types that implement this trait are expected to be cloneable via the
/// `clone_box` function.
pub trait Stream: std::io::Read + std::io::Write + Send + Sync {
    /// Clones the stream into a boxed trait object.
    fn clone_box(&self) -> Box<dyn Stream>;
}

/// Represents a file open in a shell context.
pub enum OpenFile {
    /// The original standard input this process was started with.
    Stdin(std::io::Stdin),
    /// The original standard output this process was started with.
    Stdout(std::io::Stdout),
    /// The original standard error this process was started with.
    Stderr(std::io::Stderr),
    /// A file open for reading or writing.
    File(std::fs::File),
    /// A read end of a pipe.
    PipeReader(std::io::PipeReader),
    /// A write end of a pipe.
    PipeWriter(std::io::PipeWriter),
    /// A custom stream.
    Stream(Box<dyn Stream>),
}

/// Returns an open file that will discard all I/O.
pub fn null() -> Result<OpenFile, error::Error> {
    let file = sys::fs::open_null_file()?;
    Ok(OpenFile::File(file))
}

/// 分发可读 variant 的 read 调用; 不可读的 variant 返回错误.
macro_rules! dispatch_read {
    ($self:ident, $method:ident $(, $arg:expr)*) => {
        match $self {
            Self::Stdin(f)      => f.$method($($arg),*),
            Self::File(f)       => f.$method($($arg),*),
            Self::PipeReader(f) => f.$method($($arg),*),
            Self::Stream(s)     => s.$method($($arg),*),
            Self::Stdout(_) => Err(std::io::Error::other(
                error::ErrorKind::OpenFileNotReadable("stdout"),
            )),
            Self::Stderr(_) => Err(std::io::Error::other(
                error::ErrorKind::OpenFileNotReadable("stderr"),
            )),
            Self::PipeWriter(_) => Err(std::io::Error::other(
                error::ErrorKind::OpenFileNotReadable("pipe writer"),
            )),
        }
    };
}

/// 分发可写 variant 的 write/flush 调用; 不可写的 variant 返回错误或 Ok.
macro_rules! dispatch_write {
    (write, $self:ident $(, $arg:expr)*) => {
        match $self {
            Self::Stdout(f)     => f.write($($arg),*),
            Self::Stderr(f)     => f.write($($arg),*),
            Self::File(f)       => f.write($($arg),*),
            Self::PipeWriter(f) => f.write($($arg),*),
            Self::Stream(s)     => s.write($($arg),*),
            Self::Stdin(_) => Err(std::io::Error::other(
                error::ErrorKind::OpenFileNotWritable("stdin"),
            )),
            Self::PipeReader(_) => Err(std::io::Error::other(
                error::ErrorKind::OpenFileNotWritable("pipe reader"),
            )),
        }
    };
    (flush, $self:ident) => {
        match $self {
            Self::Stdout(f)     => f.flush(),
            Self::Stderr(f)     => f.flush(),
            Self::File(f)       => f.flush(),
            Self::PipeWriter(f) => f.flush(),
            Self::Stream(s)     => s.flush(),
            Self::Stdin(_) | Self::PipeReader(_) => Ok(()),
        }
    };
}

impl Clone for OpenFile {
    fn clone(&self) -> Self {
        self.try_clone()
            .expect("failed to duplicate open file during infallible clone")
    }
}

impl std::fmt::Display for OpenFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdin(_) => write!(f, "stdin"),
            Self::Stdout(_) => write!(f, "stdout"),
            Self::Stderr(_) => write!(f, "stderr"),
            Self::File(_) => write!(f, "file"),
            Self::PipeReader(_) => write!(f, "pipe reader"),
            Self::PipeWriter(_) => write!(f, "pipe writer"),
            Self::Stream(_) => write!(f, "stream"),
        }
    }
}

impl OpenFile {
    /// Tries to duplicate the open file.
    pub fn try_clone(&self) -> Result<Self, std::io::Error> {
        let result = match self {
            Self::Stdin(_) => std::io::stdin().into(),
            Self::Stdout(_) => std::io::stdout().into(),
            Self::Stderr(_) => std::io::stderr().into(),
            Self::File(f) => f.try_clone()?.into(),
            Self::PipeReader(f) => f.try_clone()?.into(),
            Self::PipeWriter(f) => f.try_clone()?.into(),
            Self::Stream(s) => Self::Stream(s.clone_box()),
        };

        Ok(result)
    }

    pub(crate) fn is_dir(&self) -> bool {
        match self {
            Self::Stdin(_) | Self::Stdout(_) | Self::Stderr(_) => false,
            Self::File(file) => file.metadata().is_ok_and(|m| m.is_dir()),
            Self::PipeReader(_) | Self::PipeWriter(_) | Self::Stream(_) => false,
        }
    }

    /// Checks if the open file is associated with a terminal.
    pub fn is_terminal(&self) -> bool {
        match self {
            Self::Stdin(f) => f.is_terminal(),
            Self::Stdout(f) => f.is_terminal(),
            Self::Stderr(f) => f.is_terminal(),
            Self::File(f) => f.is_terminal(),
            Self::PipeReader(_) | Self::PipeWriter(_) | Self::Stream(_) => false,
        }
    }
}

impl From<std::io::Stdin> for OpenFile {
    /// Creates an `OpenFile` from standard input.
    fn from(stdin: std::io::Stdin) -> Self {
        Self::Stdin(stdin)
    }
}

impl From<std::io::Stdout> for OpenFile {
    /// Creates an `OpenFile` from standard output.
    fn from(stdout: std::io::Stdout) -> Self {
        Self::Stdout(stdout)
    }
}

impl From<std::io::Stderr> for OpenFile {
    /// Creates an `OpenFile` from standard error.
    fn from(stderr: std::io::Stderr) -> Self {
        Self::Stderr(stderr)
    }
}

impl From<std::fs::File> for OpenFile {
    fn from(file: std::fs::File) -> Self {
        Self::File(file)
    }
}

impl From<std::io::PipeReader> for OpenFile {
    fn from(reader: std::io::PipeReader) -> Self {
        Self::PipeReader(reader)
    }
}

impl From<std::io::PipeWriter> for OpenFile {
    fn from(writer: std::io::PipeWriter) -> Self {
        Self::PipeWriter(writer)
    }
}

impl From<OpenFile> for Stdio {
    fn from(open_file: OpenFile) -> Self {
        match open_file {
            OpenFile::Stdin(_) => Self::inherit(),
            OpenFile::Stdout(_) => Self::inherit(),
            OpenFile::Stderr(_) => Self::inherit(),
            OpenFile::File(f) => f.into(),
            OpenFile::PipeReader(f) => f.into(),
            OpenFile::PipeWriter(f) => f.into(),
            // NOTE: Custom streams cannot be converted to `Stdio`; we do our best here
            // and return a null device instead.
            OpenFile::Stream(_) => Self::null(),
        }
    }
}

impl std::io::Read for OpenFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        dispatch_read!(self, read, buf)
    }
}

impl std::io::Write for OpenFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        dispatch_write!(write, self, buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        dispatch_write!(flush, self)
    }
}

/// Tristate representing the an `OpenFile` entry in an `OpenFiles` structure.
#[derive(Clone, Copy)]
pub enum OpenFileEntry<'a> {
    /// File descriptor is present and has a valid associated `OpenFile`.
    Open(&'a OpenFile),
    /// File descriptor is explicitly marked as not being mapped to any `OpenFile`.
    NotPresent,
    /// File descriptor is not specified in any way; it may be provided by a
    /// parent context of some kind.
    NotSpecified,
}

/// Represents the open files in a shell context.
#[derive(Default)]
pub struct OpenFiles {
    /// Maps shell file descriptors to open files.
    files: HashMap<ShellFd, Option<OpenFile>>,
}

/// 一个只读 fd 叠加视图, 优先读取当前上下文, 再回退到父级上下文.
pub(crate) struct FdOverlay<'a> {
    overlay: &'a OpenFiles,
    fallback: &'a OpenFiles,
}

impl<'a> FdOverlay<'a> {
    /// 创建 fd 叠加视图.
    pub(crate) const fn new(overlay: &'a OpenFiles, fallback: &'a OpenFiles) -> Self {
        Self { overlay, fallback }
    }

    /// 按叠加语义解析 fd 条目.
    pub(crate) fn fd_entry(&self, fd: ShellFd) -> OpenFileEntry<'a> {
        match self.overlay.files.get(&fd) {
            Some(Some(file)) => OpenFileEntry::Open(file),
            Some(None) => OpenFileEntry::NotPresent,
            None => match self.fallback.files.get(&fd) {
                Some(Some(file)) => OpenFileEntry::Open(file),
                Some(None) => OpenFileEntry::NotPresent,
                None => OpenFileEntry::NotSpecified,
            },
        }
    }

    /// 按叠加语义查找打开文件.
    pub(crate) fn try_fd(&self, fd: ShellFd) -> Option<&'a OpenFile> {
        match self.fd_entry(fd) {
            OpenFileEntry::Open(file) => Some(file),
            OpenFileEntry::NotPresent | OpenFileEntry::NotSpecified => None,
        }
    }

    /// 检查 fd 是否在叠加视图中被明确占用或关闭.
    pub(crate) fn contains_fd(&self, fd: ShellFd) -> bool {
        !matches!(self.fd_entry(fd), OpenFileEntry::NotSpecified)
    }

    /// 遍历叠加后的所有打开文件.
    pub(crate) fn iter_fds(&self) -> impl Iterator<Item = (ShellFd, &'a OpenFile)> + 'a {
        let overlay_files = &self.overlay.files;
        let fallback_files = &self.fallback.files;

        let overlay_fds = overlay_files
            .iter()
            .filter_map(|(fd, file)| file.as_ref().map(|file| (*fd, file)));
        let fallback_fds = fallback_files
            .iter()
            .filter_map(|(fd, file)| file.as_ref().map(|file| (*fd, file)))
            .filter(move |(fd, _)| !overlay_files.contains_key(fd));

        overlay_fds.chain(fallback_fds)
    }
}

impl Clone for OpenFiles {
    fn clone(&self) -> Self {
        self.try_clone_open_files()
            .expect("failed to duplicate open files during infallible clone")
    }
}

impl OpenFiles {
    /// File descriptor used for standard input.
    pub const STDIN_FD: ShellFd = 0;
    /// File descriptor used for standard output.
    pub const STDOUT_FD: ShellFd = 1;
    /// File descriptor used for standard error.
    pub const STDERR_FD: ShellFd = 2;

    /// First file descriptor available for non-stdio files.
    const FIRST_NON_STDIO_FD: ShellFd = 3;
    /// Maximum file descriptor number allowed.
    const MAX_FD: ShellFd = 1024;

    /// Creates a new `OpenFiles` instance populated with stdin, stdout, and stderr
    /// from the host environment.
    pub(crate) fn new() -> Self {
        Self {
            files: HashMap::from([
                (Self::STDIN_FD, Some(std::io::stdin().into())),
                (Self::STDOUT_FD, Some(std::io::stdout().into())),
                (Self::STDERR_FD, Some(std::io::stderr().into())),
            ]),
        }
    }

    /// 创建一个以当前集合为优先层, 以 `fallback` 为回退层的 fd 视图.
    pub(crate) const fn overlay<'a>(&'a self, fallback: &'a OpenFiles) -> FdOverlay<'a> {
        FdOverlay::new(self, fallback)
    }

    /// 尝试复制所有打开文件条目, 保留显式关闭的 fd.
    pub fn try_clone_open_files(&self) -> Result<Self, std::io::Error> {
        let mut files = HashMap::with_capacity(self.files.len());
        for (fd, file) in &self.files {
            let cloned_file = file.as_ref().map(OpenFile::try_clone).transpose()?;
            files.insert(*fd, cloned_file);
        }
        Ok(Self { files })
    }

    /// Updates the open files from the provided iterator of (fd number, `OpenFile`) pairs.
    /// Any existing entries for the provided file descriptors will be overwritten.
    ///
    /// # Arguments
    ///
    /// * `files`: An iterator of (fd number, `OpenFile`) pairs to update the open files with.
    pub fn update_from(&mut self, files: impl Iterator<Item = (ShellFd, OpenFile)>) {
        for (fd, file) in files {
            let _ = self.files.insert(fd, Some(file));
        }
    }

    /// Retrieves the file backing standard input in this context.
    pub fn try_stdin(&self) -> Option<&OpenFile> {
        self.files.get(&Self::STDIN_FD).and_then(|f| f.as_ref())
    }

    /// Retrieves the file backing standard output in this context.
    pub fn try_stdout(&self) -> Option<&OpenFile> {
        self.files.get(&Self::STDOUT_FD).and_then(|f| f.as_ref())
    }

    /// Retrieves the file backing standard error in this context.
    pub fn try_stderr(&self) -> Option<&OpenFile> {
        self.files.get(&Self::STDERR_FD).and_then(|f| f.as_ref())
    }

    /// Tries to remove an open file by its file descriptor. If the file descriptor
    /// is not used, `None` will be returned; otherwise, the removed file will
    /// be returned.
    ///
    /// Arguments:
    ///
    /// * `fd`: The file descriptor to remove.
    pub fn remove_fd(&mut self, fd: ShellFd) -> Option<OpenFile> {
        self.files.insert(fd, None).and_then(|f| f)
    }

    /// Tries to lookup the `OpenFile` associated with a file descriptor.
    /// Returns `None` if the file descriptor is not present.
    ///
    /// Arguments:
    ///
    /// * `fd`: The file descriptor to lookup.
    pub fn try_fd(&self, fd: ShellFd) -> Option<&OpenFile> {
        self.files.get(&fd).and_then(|f| f.as_ref())
    }

    /// Tries to lookup the `OpenFile` associated with a file descriptor. Returns
    /// an `OpenFileEntry` representing the state of the file descriptor.
    ///
    /// Arguments:
    ///
    /// * `fd`: The file descriptor to lookup.
    pub fn fd_entry(&self, fd: ShellFd) -> OpenFileEntry<'_> {
        self.files
            .get(&fd)
            .map_or(OpenFileEntry::NotSpecified, |opt_file| match opt_file {
                Some(f) => OpenFileEntry::Open(f),
                None => OpenFileEntry::NotPresent,
            })
    }

    /// Checks if the given file descriptor is in use.
    pub fn contains_fd(&self, fd: ShellFd) -> bool {
        self.files.contains_key(&fd)
    }

    /// Associates the given file descriptor with the provided file. If the file descriptor
    /// is already in use, the previous file will be returned; otherwise, `None`
    /// will be returned.
    ///
    /// Arguments:
    ///
    /// * `fd`: The file descriptor to associate with the file.
    /// * `file`: The file to associate with the file descriptor.
    pub fn set_fd(&mut self, fd: ShellFd, file: OpenFile) -> Option<OpenFile> {
        self.files.insert(fd, Some(file)).and_then(|f| f)
    }

    /// Iterates over all file descriptors.
    pub fn iter_fds(&self) -> impl Iterator<Item = (ShellFd, &OpenFile)> {
        self.files
            .iter()
            .filter_map(|(fd, file)| file.as_ref().map(|f| (*fd, f)))
    }

    /// Adds a new open file, returning the assigned file descriptor.
    ///
    /// # Arguments
    ///
    /// * `file`: The open file to add.
    pub fn add(&mut self, file: OpenFile) -> Result<ShellFd, error::Error> {
        // Start searching for free file descriptors after the standard ones.
        let mut fd = Self::FIRST_NON_STDIO_FD;
        while self.files.contains_key(&fd) {
            if fd >= Self::MAX_FD {
                return Err(error::ErrorKind::TooManyOpenFiles.into());
            }

            fd += 1;
        }

        self.files.insert(fd, Some(file));
        Ok(fd)
    }
}

impl<I> From<I> for OpenFiles
where
    I: Iterator<Item = (ShellFd, OpenFile)>,
{
    fn from(iter: I) -> Self {
        let files = iter.map(|(fd, file)| (fd, Some(file))).collect();
        Self { files }
    }
}
