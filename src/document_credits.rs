use crate::model_settings;
use crate::translation_cache::{PdfId, hex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub struct TurnCredits {
    pub amount: Option<f64>,
    pub complete: bool,
}

#[derive(Default)]
pub struct DocumentCredits {
    turns: BTreeMap<String, TurnCredits>,
    unsaved: Vec<TurnCredits>,
    load_failed: bool,
}

impl DocumentCredits {
    pub fn record(&mut self, key: Option<String>, usage: TurnCredits) {
        if let Some(key) = key {
            self.turns.insert(key, usage);
        } else {
            self.unsaved.push(usage);
        }
    }

    pub fn merge(&mut self, history: Self) {
        self.turns.extend(history.turns);
        self.load_failed |= history.load_failed;
    }

    pub fn summary(&self) -> (Option<f64>, bool) {
        let mut total = None;
        let mut incomplete = self.load_failed || !self.unsaved.is_empty();
        for usage in self.turns.values().chain(&self.unsaved) {
            incomplete |= !usage.complete || usage.amount.is_none();
            if let Some(amount) = usage.amount {
                total = Some(total.unwrap_or(0.0) + amount);
            }
        }
        (total, incomplete)
    }
}

fn directory(pdf_id: PdfId) -> io::Result<PathBuf> {
    let data = model_settings::data_directory().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法定位本地应用数据目录")
    })?;
    Ok(data.join("document-credits-v1").join(hex(pdf_id)))
}

pub fn load(pdf_id: PdfId) -> DocumentCredits {
    let mut history = DocumentCredits::default();
    let result = (|| -> io::Result<()> {
        let entries = match fs::read_dir(directory(pdf_id)?) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension() != Some(OsStr::new("json")) || !entry.file_type()?.is_file() {
                continue;
            }
            let usage = File::open(&path).ok().and_then(|file| {
                serde_json::from_reader::<_, TurnCredits>(file.take(1024)).ok()
            });
            if let Some(usage) = usage.filter(|usage| usage.amount.is_none_or(|value| value.is_finite() && value >= 0.0)) {
                history.record(Some(entry.file_name().to_string_lossy().into_owned()), usage);
            } else {
                history.load_failed = true;
            }
        }
        Ok(())
    })();
    history.load_failed |= result.is_err();
    history
}

pub fn save(pdf_id: PdfId, usage: TurnCredits) -> io::Result<String> {
    let directory = directory(pdf_id)?;
    fs::create_dir_all(&directory)?;
    let mut file = tempfile::Builder::new().prefix("turn-").rand_bytes(16).tempfile_in(&directory)?;
    serde_json::to_writer(file.as_file_mut(), &usage)?;
    file.as_file().sync_data()?;
    let path = file.path().with_extension("json");
    file.persist_noclobber(&path)?;
    Ok(path.file_name().expect("turn file has a name").to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_cached_and_live_records_without_counting_a_turn_twice() {
        let settled = TurnCredits { amount: Some(2.5), complete: true };
        let mut live = DocumentCredits::default();
        live.record(Some("turn-1.json".into()), settled);
        let mut disk = DocumentCredits::default();
        disk.record(Some("turn-1.json".into()), settled);
        disk.record(Some("turn-2.json".into()), TurnCredits { amount: Some(0.0), complete: true });
        live.merge(disk);
        assert_eq!(live.summary(), (Some(2.5), false));

        live.record(Some("turn-3.json".into()), TurnCredits::default());
        assert_eq!(live.summary(), (Some(2.5), true));
    }
}
