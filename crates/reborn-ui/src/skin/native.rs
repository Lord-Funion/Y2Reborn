//! Optional typography and cover shape for Reborn's native screens and fallback components.
use super::{draw::Writer, FontRef, Skin};
use crate::{canvas::Canvas, text::Face};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Spec {
    title: Option<String>,
    body: Option<String>,
    caption: Option<String>,
    cover_radius: Option<f32>,
    corner_scale: Option<f32>,
}

#[derive(Default)]
pub(crate) struct Style {
    title: Option<usize>,
    body: Option<usize>,
    caption: Option<usize>,
    pub cover_radius: Option<f32>,
    pub corner_scale: Option<f32>,
}

impl Spec {
    pub fn compile(self, fonts: &HashMap<String, usize>) -> Result<Style, String> {
        let resolve = |name: Option<String>| {
            name.map(|name| {
                fonts
                    .get(&name)
                    .copied()
                    .ok_or_else(|| format!("native: no font named {name}"))
            })
            .transpose()
        };
        if self
            .cover_radius
            .is_some_and(|r| !r.is_finite() || !(0. ..=0.25).contains(&r))
        {
            return Err("native.cover_radius must be between 0 and 0.25 of the cover edge".into());
        }
        if self
            .corner_scale
            .is_some_and(|r| !r.is_finite() || !(0. ..=2.).contains(&r))
        {
            return Err("native.corner_scale must be between 0 and 2".into());
        }
        Ok(Style {
            title: resolve(self.title)?,
            body: resolve(self.body)?,
            caption: resolve(self.caption)?,
            cover_radius: self.cover_radius,
            corner_scale: self.corner_scale,
        })
    }
}

impl Skin {
    fn native_writer(&self, face: Face, tracking: f32) -> Option<(Writer<'_>, f32)> {
        let index = match face {
            Face::R11 | Face::R13 | Face::S10 => self.native_style.caption,
            Face::R15 | Face::R17 | Face::L23 | Face::L44 => self.native_style.body,
            _ => self.native_style.title,
        }?;
        let scale = face.px() / self.fonts.get(index)?.px;
        Some((
            Writer {
                font: FontRef::Skin(index),
                fonts: &self.fonts,
                tracking: tracking / scale,
            },
            scale,
        ))
    }

    pub(crate) fn native_width(&self, text: &str, face: Face, tracking: f32) -> Option<f32> {
        let (writer, scale) = self.native_writer(face, tracking)?;
        Some(writer.width(text) * scale)
    }

    pub(crate) fn native_fit(&self, text: &str, face: Face, max: f32) -> Option<String> {
        let (writer, scale) = self.native_writer(face, 0.)?;
        Some(writer.fit(text, max / scale))
    }

    pub(crate) fn native_wrap(
        &self,
        text: &str,
        face: Face,
        max: f32,
        lines: usize,
    ) -> Option<Vec<String>> {
        let (writer, scale) = self.native_writer(face, 0.)?;
        Some(writer.wrap(text, max / scale, lines))
    }

    pub(crate) fn native_text(
        &self,
        c: &mut Canvas,
        position: [f32; 2],
        text: &str,
        face: Face,
        color: u32,
        tracking: f32,
    ) -> Option<f32> {
        let [x, baseline] = position;
        let (writer, scale) = self.native_writer(face, tracking)?;
        let start = c.draw.len();
        let color = super::draw::remap_color(color, &self.palette, c.native_accent);
        writer.draw(c, x, baseline, text, color);
        let middle = baseline - face.mid_offset();
        for q in &mut c.draw[start..] {
            q.x = x + (q.x - x) * scale;
            q.y = middle + writer.mid() * scale + (q.y - baseline) * scale;
            q.w *= scale;
            q.h *= scale;
        }
        Some(writer.width(text) * scale)
    }
}
