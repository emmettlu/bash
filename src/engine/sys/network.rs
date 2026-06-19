pub(crate) fn get_hostname() -> std::io::Result<std::ffi::OsString> {
    crate::engine::sys::hostname::get()
}
