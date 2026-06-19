pub(crate) fn get() -> std::io::Result<std::ffi::OsString> {
    std::env::var_os("COMPUTERNAME")
        .or_else(|| std::env::var_os("HOSTNAME"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "hostname not found"))
}
