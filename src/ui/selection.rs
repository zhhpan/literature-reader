use super::{Painter, View, clip_rect, color, contains, rc};
use std::ops::Range;
use windows_sys::Win32::Foundation::{POINT, RECT};

pub(super) struct TextLine {
    pub bounds: RECT,
    pub stops: Vec<(i32, usize)>,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TextPart {
    pub row: usize,
    pub translated: bool,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct TextPosition {
    part: TextPart,
    byte: usize,
}

struct VisibleLine {
    part: TextPart,
    line: TextLine,
    hit: RECT,
}

#[derive(Default)]
pub(super) struct Selection {
    origin: Option<TextPosition>,
    extent: Option<TextPosition>,
    drag: Option<POINT>,
    viewport: Option<RECT>,
    lines: Vec<VisibleLine>,
}

impl Selection {
    pub(super) fn begin_frame(&mut self, body: RECT) {
        self.viewport = Some(body);
        self.lines.clear();
    }

    pub(super) fn draw_lines(&mut self, p: &Painter, part: TextPart, lines: Vec<TextLine>, clip: RECT) {
        let range = self.byte_range(part);
        for line in lines {
            let hit = clip_rect(line.bounds, clip);
            if hit.left >= hit.right || hit.top >= hit.bottom || line.stops.is_empty() {
                continue;
            }
            if let Some(range) = &range
                && let Some((left, right)) = selected_extent(&line.stops, range.clone())
            {
                let fill = clip_rect(rc(left, hit.top, right, hit.bottom), hit);
                if fill.left < fill.right {
                    p.fill(fill, color(184, 217, 246));
                }
            }
            self.lines.push(VisibleLine { part, line, hit });
        }
    }

    fn range(&self) -> Option<(TextPosition, TextPosition)> {
        let (origin, extent) = self.origin.zip(self.extent)?;
        (origin != extent).then(|| (origin.min(extent), origin.max(extent)))
    }

    fn byte_range(&self, part: TextPart) -> Option<Range<usize>> {
        let (first, last) = self.range()?;
        if part < first.part || part > last.part {
            return None;
        }
        let start = if part == first.part { first.byte } else { 0 };
        let end = if part == last.part { last.byte } else { usize::MAX };
        Some(start..end)
    }

    fn hit(&self, point: POINT) -> Option<&VisibleLine> {
        self.lines.iter().find(|entry| contains(entry.hit, point))
    }

    fn nearest(&self, point: POINT) -> Option<&VisibleLine> {
        let distance = |entry: &VisibleLine| {
            (entry.hit.top - point.y).max(point.y - entry.hit.bottom.saturating_sub(1)).max(0)
        };
        self.lines.iter().min_by_key(|entry| distance(entry))
    }
}

impl VisibleLine {
    fn at(&self, x: i32) -> TextPosition {
        TextPosition { part: self.part, byte: nearest_byte(&self.line.stops, x) }
    }
}

impl View {
    pub(super) fn text_at(&self, point: POINT) -> bool {
        !self.pdf_view && self.selection.hit(point).is_some()
    }

    pub(super) fn begin_text_selection(&mut self, point: POINT) -> bool {
        if self.pdf_view || self.hit(point).is_some() {
            return false;
        }
        let Some(entry) = self.selection.hit(point) else { return false };
        let caret = entry.at(point.x);
        self.selection.origin = Some(caret);
        self.selection.extent = Some(caret);
        self.selection.drag = Some(point);
        true
    }

    pub(super) fn text_dragging(&self) -> bool {
        self.selection.drag.is_some()
    }

    pub(super) fn drag_text_selection(&mut self, point: POINT) -> bool {
        if !self.text_dragging() {
            return false;
        }
        self.selection.drag = Some(point);
        let Some(entry) = self.selection.nearest(point) else { return false };
        let target = Some(entry.at(point.x));
        let changed = target != self.selection.extent;
        self.selection.extent = target;
        changed
    }

    pub(super) fn finish_text_selection(&mut self, point: POINT) -> bool {
        if !self.text_dragging() {
            return false;
        }
        self.drag_text_selection(point);
        self.cancel_text_drag();
        true
    }

    pub(super) fn cancel_text_drag(&mut self) -> bool {
        self.selection.drag.take().is_some()
    }

    pub(super) fn clear_text_selection(&mut self) -> bool {
        let changed = self.selection.origin.take().is_some();
        self.selection.extent = None;
        self.selection.drag = None;
        changed
    }

    pub(super) fn scroll_text_selection(&mut self) -> bool {
        let Some((point, viewport)) = self.selection.drag.zip(self.selection.viewport) else { return false };
        let outside = if point.y < viewport.top {
            point.y - viewport.top
        } else if point.y >= viewport.bottom {
            point.y - viewport.bottom + 1
        } else {
            return false;
        };
        let step = outside.signum() * (outside.abs() / 3).clamp(self.s(8), self.s(64));
        let before = self.scroll;
        let changed = self.drag_text_selection(point);
        self.scroll(self.scroll.saturating_add(step));
        changed || self.scroll != before
    }

    pub(super) fn select_all_text(&mut self) -> bool {
        if self.pdf_view {
            return false;
        }
        let mut text = (0..self.rows.len()).flat_map(text_parts)
            .filter_map(|part| self.text_for_part(part).filter(|text| !text.is_empty()).map(|text| (part, text.len())));
        let Some((first, first_len)) = text.next() else { return false };
        let (last, last_len) = text.next_back().unwrap_or((first, first_len));
        self.selection.origin = Some(TextPosition { part: first, byte: 0 });
        self.selection.extent = Some(TextPosition { part: last, byte: last_len });
        self.selection.drag = None;
        true
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        let (first, last) = self.selection.range()?;
        let pieces: Vec<_> = (first.part.row..=last.part.row).flat_map(text_parts).filter_map(|part| {
            let text = self.text_for_part(part)?;
            let range = self.selection.byte_range(part)?;
            let selected = text.get(range.start..range.end.min(text.len()))?;
            (!selected.is_empty()).then_some(selected)
        }).collect();
        (!pieces.is_empty()).then(|| pieces.join("\r\n\r\n"))
    }

    pub(super) fn selected_text_at(&self, point: POINT) -> Option<String> {
        let entry = self.selection.hit(point)?;
        let range = self.selection.byte_range(entry.part)?;
        let (left, right) = selected_extent(&entry.line.stops, range)?;
        (left..right).contains(&point.x).then(|| self.selected_text()).flatten()
    }

    fn text_for_part(&self, part: TextPart) -> Option<&str> {
        let document = self.reader.document.as_ref()?;
        let content = self.rows.get(part.row)?.content;
        if part.translated {
            let index = content.translated_block(document)?;
            self.reader.translations.get(index)?.as_deref()
        } else {
            content.original_text(document)
        }
    }
}

fn text_parts(row: usize) -> [TextPart; 2] {
    [TextPart { row, translated: false }, TextPart { row, translated: true }]
}

fn nearest_byte(stops: &[(i32, usize)], x: i32) -> usize {
    let i = stops.partition_point(|&(stop, _)| stop < x);
    match (i.checked_sub(1).and_then(|left| stops.get(left)), stops.get(i)) {
        (Some(&(left_x, left)), Some(&(right_x, right))) => {
            if x - left_x < right_x - x { left } else { right }
        }
        (Some(&(_, byte)), None) | (None, Some(&(_, byte))) => byte,
        (None, None) => 0,
    }
}

fn selected_extent(stops: &[(i32, usize)], range: Range<usize>) -> Option<(i32, i32)> {
    let start = range.start.max(stops.first()?.1);
    let end = range.end.min(stops.last()?.1);
    if start >= end {
        return None;
    }
    let first = stops.partition_point(|&(_, byte)| byte <= start).saturating_sub(1);
    let last = stops.partition_point(|&(_, byte)| byte < end);
    Some((stops[first].0, stops.get(last)?.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_multibyte_selection_and_whole_formula_hits() {
        let stops = [(10, 0), (20, 3), (48, 11), (62, 14)];
        assert_eq!(nearest_byte(&stops, 14), 0);
        assert_eq!(nearest_byte(&stops, 25), 3);
        assert_eq!(nearest_byte(&stops, 45), 11);
        assert_eq!(selected_extent(&stops, 3..11), Some((20, 48)));
        assert_eq!(selected_extent(&stops, 11..usize::MAX), Some((48, 62)));
        assert_eq!(selected_extent(&stops, 14..17), None);
    }
}
