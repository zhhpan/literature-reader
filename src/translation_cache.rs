use crate::model_settings;
use crate::translation::{TranslatedText, TranslationStyle};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::{
    BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
    BCryptFinishHash, BCryptHash, BCryptHashData,
};

pub type PdfId = [u8; 32];

// Version the cache when the translation format or interpretation changes.
const CACHE_DIRECTORY: &str = "translations-v1";
const MAX_FRAGMENT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct Cache {
    directory: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct CachedRow {
    index: usize,
    source_id: [u8; 32],
    text: String,
    formulas: Vec<Range<usize>>,
}

impl Cache {
    pub fn new(pdf_id: PdfId, model: &str) -> io::Result<Self> {
        let data = model_settings::data_directory().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "无法定位本地应用数据目录")
        })?;
        let model_id = sha256(model.as_bytes())?;
        let directory = data.join(CACHE_DIRECTORY).join(hex(pdf_id)).join(hex(model_id));
        Ok(Self { directory })
    }

    pub fn load<'a>(
        &self,
        block_count: usize,
        source: impl Fn(usize) -> Option<(TranslationStyle, &'a str)>,
    ) -> io::Result<Vec<(usize, TranslatedText)>> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension() == Some(OsStr::new("json")) && entry.file_type()?.is_file() {
                paths.push(path);
            }
        }
        paths.sort_unstable();

        let mut rows = BTreeMap::new();
        let mut displaced = HashMap::new();
        for path in paths {
            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let mut bytes = Vec::new();
            file.take(MAX_FRAGMENT_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_FRAGMENT_BYTES {
                continue;
            }
            let Ok(fragment) = serde_json::from_slice::<Vec<CachedRow>>(&bytes) else { continue };
            for row in fragment {
                if !valid_translation(&row.text, &row.formulas) {
                    continue;
                }
                let translation = TranslatedText { text: row.text, formulas: row.formulas };
                if let Some((style, text)) = source(row.index)
                    && source_id(style, text)? == row.source_id
                {
                    rows.entry(row.index).or_insert(translation);
                } else {
                    displaced.entry(row.source_id).or_insert(translation);
                }
            }
        }
        if !displaced.is_empty() {
            let mut destinations = HashMap::new();
            for index in 0..block_count {
                if let Some((style, text)) = source(index) {
                    let id = source_id(style, text)?;
                    if displaced.contains_key(&id) {
                        destinations.entry(id).and_modify(|position| *position = None).or_insert(Some(index));
                    }
                }
            }
            // Layout fixes can change row indexes. Reuse an unchanged source only at an unambiguous position.
            for (id, index) in destinations {
                if let Some(index) = index
                    && let Some(translation) = displaced.remove(&id)
                {
                    rows.entry(index).or_insert(translation);
                }
            }
        }
        Ok(rows.into_iter().collect())
    }

    pub fn save<'a>(
        &self,
        rows: &[(usize, TranslatedText)],
        source: impl Fn(usize) -> Option<(TranslationStyle, &'a str)>,
    ) -> io::Result<()> {
        let mut fragment = Vec::with_capacity(rows.len());
        for (index, translation) in rows {
            let Some((style, text)) = source(*index) else { continue };
            if valid_translation(&translation.text, &translation.formulas) {
                fragment.push(CachedRow {
                    index: *index,
                    source_id: source_id(style, text)?,
                    text: translation.text.clone(),
                    formulas: translation.formulas.clone(),
                });
            }
        }
        if fragment.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(&self.directory)?;
        let mut file = tempfile::Builder::new().prefix("batch-").rand_bytes(16).tempfile_in(&self.directory)?;
        {
            let mut output = BufWriter::new(file.as_file_mut());
            serde_json::to_writer(&mut output, &fragment)?;
            output.flush()?;
        }
        if file.as_file().metadata()?.len() > MAX_FRAGMENT_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "待保存的译文批次过大"));
        }
        file.as_file().sync_data()?;
        // Readers only see completed fragments, and concurrent saves never replace an existing one.
        let destination = file.path().with_extension("json");
        file.persist_noclobber(destination)?;
        Ok(())
    }
}

fn valid_translation(text: &str, formulas: &[Range<usize>]) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let mut end = 0;
    for range in formulas {
        if range.start < end || range.start >= range.end || text.get(range.clone()).is_none() {
            return false;
        }
        end = range.end;
    }
    true
}

pub fn pdf_id(path: &Path) -> io::Result<PdfId> {
    let mut input = File::open(path)?;
    let hash = Sha256::new()?;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            return hash.finish();
        }
        hash.update(&buffer[..read])?;
    }
}

fn source_id(style: TranslationStyle, text: &str) -> io::Result<[u8; 32]> {
    let tag = match style {
        TranslationStyle::Prose => 0,
        TranslationStyle::Bibliography => 1,
    };
    let hash = Sha256::new()?;
    hash.update(&[tag])?;
    hash.update(text.as_bytes())?;
    hash.finish()
}

fn sha256(bytes: &[u8]) -> io::Result<[u8; 32]> {
    let mut digest = [0; 32];
    check(unsafe {
        BCryptHash(BCRYPT_SHA256_ALG_HANDLE, null(), 0, bytes.as_ptr(), byte_count(bytes)?,
            digest.as_mut_ptr(), digest.len() as u32)
    })?;
    Ok(digest)
}

fn byte_count(bytes: &[u8]) -> io::Result<u32> {
    u32::try_from(bytes.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SHA-256 输入过长"))
}

pub(crate) fn hex(digest: [u8; 32]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(digest.len() * 2);
    for byte in digest {
        result.push(DIGITS[usize::from(byte >> 4)] as char);
        result.push(DIGITS[usize::from(byte & 15)] as char);
    }
    result
}

struct Sha256(BCRYPT_HASH_HANDLE);

impl Sha256 {
    fn new() -> io::Result<Self> {
        let mut handle = null_mut();
        check(unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut handle, null_mut(), 0, null(), 0, 0) })?;
        Ok(Self(handle))
    }

    fn update(&self, bytes: &[u8]) -> io::Result<()> {
        check(unsafe { BCryptHashData(self.0, bytes.as_ptr(), byte_count(bytes)?, 0) })
    }

    fn finish(self) -> io::Result<[u8; 32]> {
        let mut digest = [0; 32];
        check(unsafe { BCryptFinishHash(self.0, digest.as_mut_ptr(), digest.len() as u32, 0) })?;
        Ok(digest)
    }
}

impl Drop for Sha256 {
    fn drop(&mut self) {
        unsafe { BCryptDestroyHash(self.0) };
    }
}

fn check(status: i32) -> io::Result<()> {
    if status < 0 {
        return Err(io::Error::other(format!("Windows SHA-256 计算失败 ({status:#010x})")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_id_hashes_file_contents() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let first = directory.path().join("first.pdf");
        let second = directory.path().join("second.pdf");
        fs::write(&first, b"abc").unwrap();
        fs::write(&second, b"abc").unwrap();
        let first_id = pdf_id(&first).unwrap();
        assert_eq!(first_id, pdf_id(&second).unwrap());
        assert_eq!(hex(first_id), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        fs::write(&second, b"abcd").unwrap();
        assert_ne!(first_id, pdf_id(&second).unwrap());
    }

    #[test]
    fn restores_each_saved_fragment_for_its_matching_source_and_style() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let cache = Cache { directory: directory.path().join("fragments") };
        let source = |index| match index {
            2 => Some((TranslationStyle::Prose, "Where x^2.")),
            4 => Some((TranslationStyle::Bibliography, "Reference")),
            _ => None,
        };
        assert!(cache.load(5, source).unwrap().is_empty());
        cache.save(&[(2, TranslatedText { text: "设x^2".into(), formulas: vec![3..6] })], source).unwrap();
        cache.save(&[(4, TranslatedText { text: "参考文献".into(), formulas: Vec::new() })], source).unwrap();
        fs::write(cache.directory.join("broken.json"), b"[").unwrap();
        let loaded = cache.load(5, source).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].0, 2);
        assert_eq!(loaded[0].1.text, "设x^2");
        assert_eq!(loaded[0].1.formulas, vec![3..6]);
        assert_eq!(loaded[1].0, 4);
        assert!(cache.load(5, |index| match index {
            2 => Some((TranslationStyle::Prose, "Where x^3.")),
            4 => Some((TranslationStyle::Prose, "Reference")),
            _ => None,
        }).unwrap().is_empty());
        assert!(!valid_translation("设x^2", &[1..6]));
    }

    #[test]
    fn remaps_only_unique_unchanged_sources_and_keeps_exact_matches() {
        let directory = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let cache = Cache { directory: directory.path().join("fragments") };
        let prior = |index| match index {
            0 => Some((TranslationStyle::Prose, "Same source")),
            1 => Some((TranslationStyle::Prose, "Repeated source")),
            2 => Some((TranslationStyle::Prose, "Edited source")),
            _ => None,
        };
        let current = |index| match index {
            0 | 2 => Some((TranslationStyle::Prose, "Repeated source")),
            3 => Some((TranslationStyle::Prose, "Same source")),
            4 => Some((TranslationStyle::Prose, "New source")),
            _ => None,
        };
        let translations = ["原译文", "重复译文", "旧版译文"].into_iter().enumerate()
            .map(|(index, text)| (index, TranslatedText { text: text.into(), formulas: Vec::new() }))
            .collect::<Vec<_>>();
        cache.save(&translations, prior).unwrap();
        let loaded = cache.load(5, current).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!((loaded[0].0, loaded[0].1.text.as_str()), (3, "原译文"));

        cache.save(&[(3, TranslatedText { text: "当前译文".into(), formulas: Vec::new() })], current).unwrap();
        let loaded = cache.load(5, current).unwrap();
        assert_eq!((loaded[0].0, loaded[0].1.text.as_str()), (3, "当前译文"));
    }
}
