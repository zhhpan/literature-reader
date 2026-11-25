use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

#[cfg(windows)]
pub fn data_directory() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    directory.is_absolute().then(|| directory.join("LiteratureReader"))
}

#[cfg(target_os = "linux")]
pub fn data_directory() -> Option<PathBuf> {
    let absolute = |name| std::env::var_os(name).map(PathBuf::from).filter(|path| path.is_absolute());
    absolute("XDG_DATA_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".local/share")))
        .map(|directory| directory.join("literature-reader"))
}

fn model_path() -> Option<PathBuf> {
    Some(data_directory()?.join("model.json"))
}

pub fn load_model() -> Option<String> {
    let file = File::open(model_path()?).ok()?;
    let id: String = serde_json::from_reader(file.take(4096)).ok()?;
    (!id.is_empty()).then_some(id)
}

pub fn save_model(id: &str) -> io::Result<()> {
    let path = model_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法定位本地应用数据目录")
    })?;
    let directory = path.parent().expect("model path includes a directory");
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(file.as_file_mut(), id)?;
    file.persist(path)?;
    Ok(())
}
