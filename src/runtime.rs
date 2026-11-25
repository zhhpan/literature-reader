use std::path::{Path, PathBuf};

pub fn node() -> PathBuf {
    #[cfg(windows)]
    let (path, name) = ("runtime/node/node.exe", "node.exe");
    #[cfg(target_os = "linux")]
    let (path, name) = ("runtime/node/node", "node");
    executable(path, name)
}

pub fn poppler(program: &str) -> PathBuf {
    #[cfg(windows)]
    let path = format!("runtime/poppler/Library/bin/{program}.exe");
    #[cfg(target_os = "linux")]
    let path = format!("runtime/poppler/bin/{program}");
    executable(path, program)
}

pub fn bridge(exe: &Path) -> Option<PathBuf> {
    let script = Path::new("bridge/bridge.mjs");
    installed_file(exe, script).or_else(|| {
        // A source checkout has its bridge above Cargo's target directory.
        exe.ancestors().skip(2).take(2).map(|directory| directory.join(script))
            .find(|path| path.is_file())
    })
}

fn executable(installed_path: impl AsRef<Path>, name: &str) -> PathBuf {
    std::env::current_exe().ok()
        .and_then(|exe| installed_file(&exe, installed_path.as_ref()))
        .unwrap_or_else(|| PathBuf::from(name))
}

fn installed_file(exe: &Path, relative_path: &Path) -> Option<PathBuf> {
    let directory = exe.parent()?;
    let adjacent = directory.join(relative_path);
    if adjacent.is_file() {
        return Some(adjacent);
    }
    #[cfg(target_os = "linux")]
    {
        let private = directory.parent()?.join("lib/literature-reader").join(relative_path);
        if private.is_file() {
            return Some(private);
        }
    }
    None
}
