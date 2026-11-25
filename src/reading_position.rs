use crate::model_settings;
use crate::translation_cache::{PdfId, hex};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

const POSITION_DIRECTORY: &str = "reading-positions-v1";
const MAX_POSITION_BYTES: u64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Content,
    TranslatedList,
    ReferenceHeading,
    Reference,
    TranslatedReferenceHeading,
    TranslatedReference,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "view", rename_all = "snake_case")]
pub enum ReadingPosition {
    Bilingual { block: usize, page: usize, left: f32, top: f32, section: Section },
    Original { page: usize, offset: i32 },
}

impl ReadingPosition {
    fn is_valid(self) -> bool {
        match self {
            Self::Bilingual { page, left, top, .. } => page > 0 && left.is_finite() && top.is_finite(),
            Self::Original { page, offset } => page > 0 && offset >= 0,
        }
    }
}

fn position_path(id: PdfId) -> Option<PathBuf> {
    let directory = model_settings::data_directory()?.join(POSITION_DIRECTORY);
    Some(directory.join(hex(id)).with_extension("json"))
}

pub fn load(id: PdfId) -> Option<ReadingPosition> {
    let file = File::open(position_path(id)?).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_POSITION_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_POSITION_BYTES {
        return None;
    }
    let position: ReadingPosition = serde_json::from_slice(&bytes).ok()?;
    position.is_valid().then_some(position)
}

pub fn save(id: PdfId, position: ReadingPosition) -> io::Result<()> {
    if !position.is_valid() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "阅读位置无效"));
    }
    let path = position_path(id).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "无法定位本地应用数据目录")
    })?;
    let directory = path.parent().expect("position path includes a directory");
    fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(file.as_file_mut(), &position)?;
    file.persist(path)?;
    Ok(())
}
