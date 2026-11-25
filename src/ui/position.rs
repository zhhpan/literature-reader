use super::{RowContent, RowKind, View, rescale};
use crate::pdf::{BlockKind, Bounds, Document};
use crate::reading_position::{ReadingPosition, Section};
use std::time::{Duration, Instant};

const SAVE_DELAY: Duration = Duration::from_millis(650);

#[derive(Default)]
pub(super) struct PositionState {
    pending: Option<ReadingPosition>,
    saved: Option<ReadingPosition>,
    save_after: Option<Instant>,
}

impl PositionState {
    pub(super) fn take_original_scroll(&mut self, dpi: u32, maximum: i32) -> Option<i32> {
        let ReadingPosition::Original { offset, .. } = self.pending.take()? else { return None };
        let pixels = (i64::from(offset) * i64::from(dpi) + 48) / 96;
        Some(pixels.clamp(0, i64::from(maximum)) as i32)
    }
}

impl View {
    pub(super) fn load_reading_position(&mut self) {
        let Some(position) = self.reader.take_reading_position() else { return };
        self.reading_position.saved = Some(position);
        self.reading_position.pending = Some(position);
        if let ReadingPosition::Original { page, .. } = position
            && let Some(document) = &self.reader.document
        {
            self.pdf_view = true;
            self.reader.page = page.clamp(1, document.pages.max(1));
        }
    }

    pub(super) fn restore_bilingual_position(&mut self) {
        let Some(position @ ReadingPosition::Bilingual { .. }) = self.reading_position.pending.take() else {
            return;
        };
        if let Some(document) = &self.reader.document
            && let Some(index) = anchor_row(self.rows.iter().map(|row| row.content), document, position)
        {
            let row = &self.rows[index];
            let index = if row.height == 0 && matches!(row.content.kind, RowKind::ListTranslation { .. }) {
                self.source_rows[row.content.block].unwrap_or(index)
            } else {
                index
            };
            self.scroll = if index == 0 { 0 } else { self.rows[index].y };
        }
    }

    pub(super) fn mark_reading_position_changed(&mut self) {
        if self.reader.document.is_none() {
            return;
        }
        self.reading_position.pending = None;
        self.reading_position.save_after = Some(Instant::now() + SAVE_DELAY);
    }

    pub(super) fn save_reading_position_if_due(&mut self) {
        if self.reading_position.save_after.is_some_and(|deadline| Instant::now() >= deadline) {
            self.flush_reading_position();
        }
    }

    pub(super) fn flush_reading_position(&mut self) {
        if self.reading_position.save_after.take().is_none() {
            return;
        }
        if let Some(position) = self.current_reading_position()
            && self.reading_position.saved != Some(position)
            && self.reader.save_reading_position(position)
        {
            self.reading_position.saved = Some(position);
        }
    }

    fn current_reading_position(&self) -> Option<ReadingPosition> {
        let document = self.reader.document.as_ref()?;
        if self.pdf_view {
            return Some(ReadingPosition::Original {
                page: self.reader.page,
                offset: rescale(self.scroll, self.dpi, 96),
            });
        }
        let row = self.rows.get(self.first_visible_row()).or_else(|| self.rows.last())?;
        let block = document.blocks.get(row.content.block)?;
        Some(ReadingPosition::Bilingual {
            block: row.content.block,
            page: block.page,
            left: block.bounds.left,
            top: block.bounds.top,
            section: row.content.kind.reading_section(),
        })
    }
}

pub(super) fn anchor_row(
    rows: impl Iterator<Item = RowContent>,
    document: &Document,
    position: ReadingPosition,
) -> Option<usize> {
    let ReadingPosition::Bilingual { block, page, left, top, section } = position else { return None };
    rows.enumerate().filter_map(|(index, row)| {
        let source = document.blocks.get(row.block)?;
        let distance = |(source_page, bounds): (usize, Bounds)| {
            let mut distance = (bounds.left - left).abs() + (bounds.top - top).abs();
            if source.kind == BlockKind::Paragraph && source_page == page
                && (bounds.left - 0.5..=bounds.right).contains(&left)
                && (bounds.top - 0.5..bounds.bottom).contains(&top)
            {
                distance = distance.min(0.5);
            }
            (source_page.abs_diff(page), distance)
        };
        let (pages, distance) = std::iter::once((source.page, source.bounds))
            .chain(document.paragraph_continuations.get(&row.block).into_iter().flatten().copied())
            .map(distance).min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)))?;
        let rank = (pages, row.kind.reading_section() != section);
        Some((index, rank, distance, row.block.abs_diff(block)))
    }).min_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)).then_with(|| a.3.cmp(&b.3))
    }).map(|(index, ..)| index)
}

impl RowKind {
    fn reading_section(self) -> Section {
        match self {
            Self::Text | Self::Media { .. } | Self::ListSource { .. } => Section::Content,
            Self::ListTranslation { .. } => Section::TranslatedList,
            Self::ReferenceHeading => Section::ReferenceHeading,
            Self::ReferenceSource => Section::Reference,
            Self::ReferenceTranslationHeading => Section::TranslatedReferenceHeading,
            Self::ReferenceTranslation => Section::TranslatedReference,
        }
    }
}
