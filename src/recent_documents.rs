use crate::model_settings;
use crate::translation_cache::{PdfId, hex};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

const DIRECTORY: &str = "recent-documents-v1";
const MAX_ENTRIES: usize = 12;
const MAX_RECORD_BYTES: u64 = 256 * 1024;

#[derive(Deserialize, Serialize)]
pub struct RecentDocument {
    id: PdfId,
    #[serde(with = "windows_path")]
    pub path: PathBuf,
    pub title: String,
}

impl RecentDocument {
    fn matches(&self, other: &Self) -> bool {
        self.id == other.id || self.path == other.path
    }
}

#[derive(Default)]
pub struct RecentDocuments {
    entries: Vec<RecentDocument>,
}

impl RecentDocuments {
    pub fn load() -> Self {
        directory().map_or_else(Self::default, |path| Self::load_directory(&path))
    }

    pub fn entries(&self) -> &[RecentDocument] {
        &self.entries
    }

    pub fn record(&mut self, id: PdfId, path: &Path, title: &str) -> io::Result<()> {
        let path = std::path::absolute(path)?;
        let title = title.trim();
        let title = if title.is_empty() {
            path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned()
        } else {
            title.chars().take(2048).collect()
        };
        let entry = RecentDocument { id, path, title };
        self.entries.retain(|other| !other.matches(&entry));
        self.entries.insert(0, entry);
        self.entries.truncate(MAX_ENTRIES);

        let directory = directory().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "无法定位本地应用数据目录")
        })?;
        save_document(&directory, &self.entries[0])
    }

    fn load_directory(directory: &Path) -> Self {
        let Ok(files) = fs::read_dir(directory) else { return Self::default() };
        let mut records: Vec<_> = files.filter_map(|file| {
            let file = file.ok()?;
            let path = file.path();
            if path.extension() != Some(OsStr::new("json")) || !file.file_type().ok()?.is_file() {
                return None;
            }
            Some((file.metadata().ok()?.modified().ok()?, path))
        }).collect();
        records.sort_unstable_by(|left, right| right.cmp(left));

        let mut entries = Vec::<RecentDocument>::with_capacity(MAX_ENTRIES);
        for (_, path) in records {
            let Some(entry) = load_document(&path) else { continue };
            if entries.iter().any(|other| other.matches(&entry)) {
                continue;
            }
            entries.push(entry);
            if entries.len() == MAX_ENTRIES {
                break;
            }
        }
        Self { entries }
    }
}

fn directory() -> Option<PathBuf> {
    Some(model_settings::data_directory()?.join(DIRECTORY))
}

fn load_document(path: &Path) -> Option<RecentDocument> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return None;
    }
    let entry: RecentDocument = serde_json::from_slice(&bytes).ok()?;
    (entry.path.is_absolute() && !entry.title.trim().is_empty()
        && path.file_stem() == Some(OsStr::new(&hex(entry.id))))
        .then_some(entry)
}

fn save_document(directory: &Path, entry: &RecentDocument) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(file.as_file_mut(), entry)?;
    // A file per document preserves entries written by other reader windows.
    file.persist(directory.join(hex(entry.id)).with_extension("json"))?;
    Ok(())
}

mod windows_path {
    use super::*;

    pub fn serialize<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(path.as_os_str().encode_wide())
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<PathBuf, D::Error> {
        let path = Vec::<u16>::deserialize(deserializer)?;
        if path.contains(&0) {
            return Err(serde::de::Error::custom("文件路径包含空字符"));
        }
        Ok(OsString::from_wide(&path).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{FileTimes, OpenOptions};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn keeps_latest_record_for_a_path_and_preserves_other_documents() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let path = PathBuf::from(r"C:\文献\论文.pdf");
        let other_path = PathBuf::from(r"D:\资料\另一篇.pdf");
        let entries = [
            RecentDocument { id: [1; 32], path: path.clone(), title: "旧内容".into() },
            RecentDocument { id: [2; 32], path: other_path.clone(), title: "另一篇".into() },
            RecentDocument { id: [3; 32], path: path.clone(), title: "新内容".into() },
        ];
        for (index, entry) in entries.iter().enumerate() {
            save_document(directory.path(), entry).unwrap();
            let file = OpenOptions::new().write(true)
                .open(directory.path().join(hex(entry.id)).with_extension("json")).unwrap();
            let time = UNIX_EPOCH + Duration::from_secs(1_700_000_000 + index as u64);
            file.set_times(FileTimes::new().set_modified(time)).unwrap();
        }
        fs::write(directory.path().join("broken.json"), "{").unwrap();

        let records = RecentDocuments::load_directory(directory.path());
        assert_eq!(records.entries().len(), 2);
        assert_eq!(records.entries()[0].path, path);
        assert_eq!(records.entries()[0].title, "新内容");
        assert_eq!(records.entries()[1].path, other_path);
    }
}
