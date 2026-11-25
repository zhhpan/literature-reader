use std::path::{Path, PathBuf};

pub fn executable(installed_path: impl AsRef<Path>, name: &str) -> PathBuf {
    std::env::current_exe().ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(installed_path)))
        .filter(|exe| exe.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}
