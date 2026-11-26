//! Optional page-layout detection (DocLayout-YOLO ONNX).
//!
//! The model only supplies region boxes and classes; text still comes from
//! Poppler. This module starts as a probe so the existing extraction stays
//! untouched until the regions are validated on real papers.

use crate::pdf::{self, BlockKind};
use crate::reader::Raster;
use image::{Rgb, RgbImage};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;
use std::time::Instant;

pub const CLASSES: [&str; 10] = [
    "title", "plain text", "abandon", "figure", "figure_caption",
    "table", "table_caption", "table_footnote", "isolate_formula", "formula_caption",
];

#[derive(Clone, Debug)]
pub struct Region {
    pub class: &'static str,
    pub confidence: f32,
    /// PDF points, top-left origin.
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub struct LayoutModel {
    session: Session,
    stride: usize,
}

impl LayoutModel {
    pub fn load(path: &Path) -> Result<Self, String> {
        let builder = Session::builder().map_err(|error| format!("无法加载版面模型：{error}"))?;
        let mut builder = builder.with_intra_threads(8).map_err(|error| format!("无法配置版面模型：{error}"))?;
        let session = builder.commit_from_file(path).map_err(|error| format!("无法加载版面模型：{error}"))?;
        Ok(Self { session, stride: 32 })
    }

    /// `raster` is the page at `dpi`; boxes come back in PDF points.
    pub fn detect(&mut self, raster: &Raster, dpi: u32) -> Result<Vec<Region>, String> {
        let width = raster.width.max(1) as u32;
        let height = raster.height.max(1) as u32;
        let mut rgb = RgbImage::new(width, height);
        for (pixel, rgba) in rgb.pixels_mut().zip(raster.bgra.chunks_exact(4)) {
            *pixel = Rgb([rgba[2], rgba[1], rgba[0]]);
        }

        let target = (height as usize / self.stride) * self.stride;
        let gain = (target as f32 / height as f32).min(target as f32 / width as f32);
        let resized_w = (width as f32 * gain).round().max(1.0) as u32;
        let resized_h = (height as f32 * gain).round().max(1.0) as u32;
        let resized = image::imageops::resize(&rgb, resized_w, resized_h, image::imageops::FilterType::Triangle);
        let mut canvas = RgbImage::from_pixel(target as u32, target as u32, Rgb([114, 114, 114]));
        let offset_x = (target as i32 - resized_w as i32) / 2;
        let offset_y = (target as i32 - resized_h as i32) / 2;
        image::imageops::overlay(&mut canvas, &resized, offset_x as i64, offset_y as i64);

        let side = target;
        let mut input = vec![0f32; 3 * side * side];
        for (index, pixel) in canvas.pixels().enumerate() {
            let [r, g, b] = pixel.0;
            input[index] = b as f32 / 255.0;
            input[side * side + index] = g as f32 / 255.0;
            input[2 * side * side + index] = r as f32 / 255.0;
        }

        let prep = std::time::Instant::now();
        let input = Tensor::from_array(([1usize, 3, side, side], input))
            .map_err(|error| format!("无法创建模型输入：{error}"))?;
        eprintln!("layout preprocess {} ms", prep.elapsed().as_millis());
        let run = std::time::Instant::now();
        let outputs = self.session.run(ort::inputs!["images" => input])
            .map_err(|error| format!("版面推理失败：{error}"))?;
        eprintln!("layout run {} ms", run.elapsed().as_millis());
        let (shape, data) = outputs[0].try_extract_tensor::<f32>()
            .map_err(|error| format!("无法读取模型输出：{error}"))?;
        let dims: Vec<usize> = shape.iter().map(|&value| value as usize).collect();
        if dims.len() != 3 || dims[2] < 6 {
            return Err(format!("版面模型输出形状异常：{dims:?}"));
        }
        let scale = 72.0 / dpi as f32;
        let pad_x = (target as f32 - width as f32 * gain) / 2.0;
        let pad_y = (target as f32 - height as f32 * gain) / 2.0;
        let mut regions = Vec::new();
        for row in data.chunks_exact(dims[2]) {
            let confidence = row[4];
            if confidence < 0.25 {
                continue;
            }
            let class = CLASSES.get(row[5] as usize).copied().unwrap_or("abandon");
            let x0 = ((row[0] - pad_x) / gain).clamp(0.0, width as f32) * scale;
            let y0 = ((row[1] - pad_y) / gain).clamp(0.0, height as f32) * scale;
            let x1 = ((row[2] - pad_x) / gain).clamp(0.0, width as f32) * scale;
            let y1 = ((row[3] - pad_y) / gain).clamp(0.0, height as f32) * scale;
            if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
                continue;
            }
            regions.push(Region { class, confidence, left: x0, top: y0, right: x1, bottom: y1 });
        }
        regions.sort_by(|left, right| {
            left.top.total_cmp(&right.top).then_with(|| left.left.total_cmp(&right.left))
        });
        Ok(regions)
    }
}

/// `literature-reader --layout-probe <pdf> <page> <model.onnx> [out.txt]`
pub fn probe(args: &[String]) -> i32 {
    let (pdf, page, model, out) = match args {
        [pdf, page, model] => (pdf, page, model, None),
        [pdf, page, model, out] => (pdf, page, model, Some(out.as_str())),
        _ => {
            eprintln!("用法：literature-reader --layout-probe <pdf> <page> <model.onnx> [out.txt]");
            return 2;
        }
    };
    let mut report = String::new();
    use std::fmt::Write as _;
    let Ok(page) = page.parse::<usize>() else {
        eprintln!("页码必须是正整数");
        return 2;
    };
    let path = Path::new(pdf);
    let mut model = match LayoutModel::load(Path::new(model)) {
        Ok(model) => model,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };

    let started = Instant::now();
    let ppm = match pdf::render_page(path, page, 150) {
        Ok(ppm) => ppm,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let raster = match Raster::decode(&ppm) {
        Ok(raster) => raster,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let render_ms = started.elapsed().as_millis();
    let started = Instant::now();
    let regions = match model.detect(&raster, 150) {
        Ok(regions) => regions,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let detect_ms = started.elapsed().as_millis();
    let _ = writeln!(report, "page {page}: render {render_ms} ms, layout {detect_ms} ms, {} regions", regions.len());
    for region in &regions {
        let _ = writeln!(
            report,
            "{:16} {:.2}  ({:.0},{:.0})-({:.0},{:.0})  w={:.0} h={:.0}",
            region.class, region.confidence,
            region.left, region.top, region.right, region.bottom,
            region.right - region.left, region.bottom - region.top,
        );
    }

    match pdf::extract(path) {
        Ok(document) => {
            let blocks: Vec<_> = document.blocks.iter().enumerate()
                .filter(|(_, block)| block.page == page)
                .collect();
            let _ = writeln!(report, "current extraction: {} blocks on page {page}", blocks.len());
            for (index, block) in blocks {
                let kind = match block.kind {
                    BlockKind::Title => "title",
                    BlockKind::Author => "author",
                    BlockKind::Heading => "heading",
                    BlockKind::Paragraph => "paragraph",
                    BlockKind::Equation => "equation",
                    BlockKind::Caption => "caption",
                    BlockKind::ReferenceHeading => "reference_heading",
                    BlockKind::Reference => "reference",
                    BlockKind::Figure => "figure",
                    BlockKind::Table => "table",
                };
                let text: String = block.text.chars().take(48).collect();
                let _ = writeln!(report, "#{index} {kind:15} ({:.0},{:.0})-({:.0},{:.0}) {text}",
                    block.bounds.left, block.bounds.top, block.bounds.right, block.bounds.bottom);
            }
        }
        Err(error) => eprintln!("无法读取当前提取结果：{error}"),
    }
    if let Some(out) = out {
        if let Err(error) = std::fs::write(out, report) {
            eprintln!("无法写入报告：{error}");
            return 1;
        }
    } else {
        print!("{report}");
    }
    0
}
