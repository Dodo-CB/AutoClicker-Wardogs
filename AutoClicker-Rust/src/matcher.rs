//! Template-Matching (Ersatz für OpenCV `matchTemplate` mit Maske).
//!
//! Score = 1 - Σ M·(T-I)² / sqrt(ΣM·T² · ΣM·I²)   (entspricht TM_SQDIFF_NORMED mit Maske)
//!
//! Damit das auch in Full-HD/4K schnell ist, wird grob-zu-fein gesucht:
//!   1. Bild und Vorlage werden verkleinert, dort wird (parallel über alle Kerne) gesucht.
//!   2. Nur um die besten Kandidaten wird in voller Auflösung exakt nachgerechnet.

use rayon::prelude::*;
use std::collections::HashMap;

/// BGR, 3 Bytes pro Pixel
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

/// BGR-Vorlage + Maske (0 = ignorieren, 1 = verwenden)
pub struct Template {
    pub w: usize,
    pub h: usize,
    pub bgr: Vec<u8>,
    pub mask: Vec<u8>,
}

pub struct Found {
    pub score: f64,
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

/// Nur die sichtbaren Vorlagen-Pixel, mit Offsets passend zum Zielbild.
struct Prepared {
    offs: Vec<usize>,
    vals: Vec<[i32; 3]>,
    sum_t2: f64,
}

fn prepare(t: &Template, stride: usize) -> Prepared {
    let mut offs = Vec::new();
    let mut vals = Vec::new();
    let mut sum: u64 = 0;
    for y in 0..t.h {
        for x in 0..t.w {
            let i = y * t.w + x;
            if t.mask[i] == 0 {
                continue;
            }
            let p = i * 3;
            let (b, g, r) = (t.bgr[p] as i32, t.bgr[p + 1] as i32, t.bgr[p + 2] as i32);
            offs.push((y * stride + x) * 3);
            vals.push([b, g, r]);
            sum += (b * b + g * g + r * r) as u64;
        }
    }
    Prepared { offs, vals, sum_t2: sum as f64 }
}

#[inline]
fn score_at(img: &Image, p: &Prepared, x: usize, y: usize) -> f64 {
    let base = (y * img.w + x) * 3;
    let d = &img.data;
    let mut sd: u64 = 0;
    let mut si: u64 = 0;
    for (o, v) in p.offs.iter().zip(p.vals.iter()) {
        let i = base + *o;
        let b = d[i] as i32;
        let g = d[i + 1] as i32;
        let r = d[i + 2] as i32;
        let (db, dg, dr) = (b - v[0], g - v[1], r - v[2]);
        sd += (db * db + dg * dg + dr * dr) as u64;
        si += (b * b + g * g + r * r) as u64;
    }
    let denom = (p.sum_t2 * si as f64).sqrt();
    if denom <= 0.0 || !denom.is_finite() {
        return 0.0;
    }
    1.0 - sd as f64 / denom
}

// ---------------------------------------------------------------
//  Skalieren
// ---------------------------------------------------------------
fn downscale_image(img: &Image, f: usize) -> Image {
    let (nw, nh) = (img.w / f, img.h / f);
    let mut data = vec![0u8; nw * nh * 3];
    let area = (f * f) as u32;
    for dy in 0..nh {
        for dx in 0..nw {
            let mut acc = [0u32; 3];
            for yy in 0..f {
                let row = ((dy * f + yy) * img.w + dx * f) * 3;
                for xx in 0..f {
                    let p = row + xx * 3;
                    acc[0] += img.data[p] as u32;
                    acc[1] += img.data[p + 1] as u32;
                    acc[2] += img.data[p + 2] as u32;
                }
            }
            let o = (dy * nw + dx) * 3;
            data[o] = ((acc[0] + area / 2) / area) as u8;
            data[o + 1] = ((acc[1] + area / 2) / area) as u8;
            data[o + 2] = ((acc[2] + area / 2) / area) as u8;
        }
    }
    Image { w: nw, h: nh, data }
}

fn downscale_template(t: &Template, f: usize) -> Template {
    let (nw, nh) = (t.w / f, t.h / f);
    let mut bgr = vec![0u8; nw * nh * 3];
    let mut mask = vec![0u8; nw * nh];
    for dy in 0..nh {
        for dx in 0..nw {
            let mut cnt = 0u32;
            let mut acc = [0u32; 3];
            for yy in 0..f {
                for xx in 0..f {
                    let i = (dy * f + yy) * t.w + dx * f + xx;
                    if t.mask[i] != 0 {
                        cnt += 1;
                        acc[0] += t.bgr[i * 3] as u32;
                        acc[1] += t.bgr[i * 3 + 1] as u32;
                        acc[2] += t.bgr[i * 3 + 2] as u32;
                    }
                }
            }
            if (cnt as usize) * 2 >= f * f && cnt > 0 {
                let o = dy * nw + dx;
                mask[o] = 1;
                bgr[o * 3] = (acc[0] / cnt) as u8;
                bgr[o * 3 + 1] = (acc[1] / cnt) as u8;
                bgr[o * 3 + 2] = (acc[2] / cnt) as u8;
            }
        }
    }
    Template { w: nw, h: nh, bgr, mask }
}

/// Flächenmittelung (beim Verkleinern) bzw. bilinear (beim Vergrößern)
fn resize_template(t: &Template, nw: usize, nh: usize) -> Template {
    let sx = t.w as f32 / nw as f32;
    let sy = t.h as f32 / nh as f32;
    let shrink = sx > 1.0 || sy > 1.0;
    let mut bgr = vec![0u8; nw * nh * 3];
    let mut mask = vec![0u8; nw * nh];

    for dy in 0..nh {
        for dx in 0..nw {
            let o = dy * nw + dx;
            // Maske: nächster Nachbar
            let mx = (((dx as f32 + 0.5) * sx) as usize).min(t.w - 1);
            let my = (((dy as f32 + 0.5) * sy) as usize).min(t.h - 1);
            mask[o] = t.mask[my * t.w + mx];

            if shrink {
                let (x0, x1) = (dx as f32 * sx, (dx as f32 + 1.0) * sx);
                let (y0, y1) = (dy as f32 * sy, (dy as f32 + 1.0) * sy);
                let mut acc = [0f32; 3];
                let mut wsum = 0f32;
                let iy1 = (y1.ceil() as usize).min(t.h);
                let ix1 = (x1.ceil() as usize).min(t.w);
                for yy in (y0.floor() as usize)..iy1 {
                    let wy = (yy as f32 + 1.0).min(y1) - (yy as f32).max(y0);
                    for xx in (x0.floor() as usize)..ix1 {
                        let wx = (xx as f32 + 1.0).min(x1) - (xx as f32).max(x0);
                        let wgt = wx * wy;
                        let p = (yy * t.w + xx) * 3;
                        acc[0] += t.bgr[p] as f32 * wgt;
                        acc[1] += t.bgr[p + 1] as f32 * wgt;
                        acc[2] += t.bgr[p + 2] as f32 * wgt;
                        wsum += wgt;
                    }
                }
                if wsum > 0.0 {
                    for c in 0..3 {
                        bgr[o * 3 + c] = (acc[c] / wsum).round().clamp(0.0, 255.0) as u8;
                    }
                }
            } else {
                let fx = ((dx as f32 + 0.5) * sx - 0.5).max(0.0);
                let fy = ((dy as f32 + 0.5) * sy - 0.5).max(0.0);
                let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(t.w - 1), (y0 + 1).min(t.h - 1));
                let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
                for c in 0..3 {
                    let p00 = t.bgr[(y0 * t.w + x0) * 3 + c] as f32;
                    let p10 = t.bgr[(y0 * t.w + x1) * 3 + c] as f32;
                    let p01 = t.bgr[(y1 * t.w + x0) * 3 + c] as f32;
                    let p11 = t.bgr[(y1 * t.w + x1) * 3 + c] as f32;
                    let top = p00 + (p10 - p00) * tx;
                    let bottom = p01 + (p11 - p01) * tx;
                    bgr[o * 3 + c] = (top + (bottom - top) * ty).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Template { w: nw, h: nh, bgr, mask }
}

// ---------------------------------------------------------------
//  Suche
// ---------------------------------------------------------------
const CANDIDATES: usize = 4;

/// Grobe Suche: liefert die besten Positionen (in Grob-Koordinaten).
fn coarse_candidates(coarse: &Image, ct: &Template) -> Vec<(usize, usize)> {
    if ct.w == 0 || ct.h == 0 || coarse.w < ct.w || coarse.h < ct.h {
        return Vec::new();
    }
    let p = prepare(ct, coarse.w);
    if p.offs.is_empty() {
        return Vec::new();
    }
    let rows = coarse.h - ct.h + 1;
    let cols = coarse.w - ct.w + 1;

    let mut best_per_row: Vec<(f64, usize, usize)> = (0..rows)
        .into_par_iter()
        .map(|y| {
            let mut best = f64::MIN;
            let mut best_x = 0;
            for x in 0..cols {
                let s = score_at(coarse, &p, x, y);
                if s > best {
                    best = s;
                    best_x = x;
                }
            }
            (best, best_x, y)
        })
        .collect();

    best_per_row.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let min_dx = (ct.w / 2).max(1) as isize;
    let min_dy = (ct.h / 2).max(1) as isize;
    let mut picked: Vec<(usize, usize)> = Vec::new();
    for (_, x, y) in best_per_row {
        if picked.len() >= CANDIDATES {
            break;
        }
        let far_enough = picked.iter().all(|&(px, py)| {
            (px as isize - x as isize).abs() >= min_dx || (py as isize - y as isize).abs() >= min_dy
        });
        if far_enough {
            picked.push((x, y));
        }
    }
    picked
}

/// Sucht die Vorlage in mehreren Skalierungen und gibt den besten Treffer zurück.
pub fn find_best(frame: &Image, tpl: &Template, scales: &[f32]) -> Option<Found> {
    let mut best: Option<Found> = None;
    let mut cache: HashMap<usize, Image> = HashMap::new();

    for &scale in scales {
        let w = (tpl.w as f32 * scale).round() as usize;
        let h = (tpl.h as f32 * scale).round() as usize;
        if w < 8 || h < 8 || w > frame.w || h > frame.h {
            continue;
        }
        let t = if w == tpl.w && h == tpl.h {
            Template { w, h, bgr: tpl.bgr.clone(), mask: tpl.mask.clone() }
        } else {
            resize_template(tpl, w, h)
        };

        let f = (w.min(h) / 24).clamp(1, 6);
        if f > 1 && !cache.contains_key(&f) {
            cache.insert(f, downscale_image(frame, f));
        }
        let coarse: &Image = if f == 1 { frame } else { &cache[&f] };
        let ct = if f == 1 {
            Template { w, h, bgr: t.bgr.clone(), mask: t.mask.clone() }
        } else {
            downscale_template(&t, f)
        };

        let candidates = coarse_candidates(coarse, &ct);
        let full = prepare(&t, frame.w);
        let max_x = frame.w - w;
        let max_y = frame.h - h;

        for (cx, cy) in candidates {
            let x0 = (cx * f).saturating_sub(f + 1);
            let y0 = (cy * f).saturating_sub(f + 1);
            let x1 = (cx * f + f + 1).min(max_x);
            let y1 = (cy * f + f + 1).min(max_y);
            for y in y0..=y1.max(y0) {
                for x in x0..=x1.max(x0) {
                    if x > max_x || y > max_y {
                        continue;
                    }
                    let s = score_at(frame, &full, x, y);
                    if best.as_ref().map_or(true, |b| s > b.score) {
                        best = Some(Found { score: s, x, y, w, h });
                    }
                }
            }
        }

        // Praktisch perfekter Treffer: restliche Skalierungen sparen
        if best.as_ref().map_or(false, |b| b.score >= 0.99) {
            break;
        }
    }
    best
}
