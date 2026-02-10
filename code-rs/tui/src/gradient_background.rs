#![allow(clippy::disallowed_methods)]

use crate::card_theme::{GradientSpec, RevealVariant};
use crate::colors;
use crate::glitch_animation::{gradient_multi, mix_rgb};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Color;

#[derive(Clone, Copy, Debug)]
pub struct RevealRender {
    pub progress: f32,
    pub variant: RevealVariant,
    pub intro_light: bool,
}

pub struct GradientBackground;

impl GradientBackground {
    pub fn render(
        buf: &mut Buffer,
        area: Rect,
        gradient: &GradientSpec,
        fg: Color,
        reveal: Option<RevealRender>,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if let Some(reveal) = reveal {
            Self::render_reveal(buf, area, gradient, fg, reveal);
        } else {
            Self::render_static(buf, area, gradient, fg);
        }
    }

    pub fn render_static(buf: &mut Buffer, area: Rect, gradient: &GradientSpec, fg: Color) {
        for row in 0..area.height {
            for col in 0..area.width {
                let x = col as f32 / area.width.max(1) as f32;
                let t = (x + gradient.bias).clamp(0.0, 1.0);
                let color = colors::mix_toward(gradient.left, gradient.right, t);
                let cell = &mut buf[(area.x + col, area.y + row)];
                cell.set_symbol(" ");
                cell.set_fg(fg);
                cell.set_bg(color);
            }
        }
    }

    pub fn render_reveal(
        buf: &mut Buffer,
        area: Rect,
        gradient: &GradientSpec,
        fg: Color,
        reveal: RevealRender,
    ) {
        let clamped_progress = reveal.progress.clamp(0.0, 1.0);
        const LIGHT_REVEAL_HOLD: f32 = 0.06;
        const LIGHT_REVEAL_FADE_END: f32 = 0.32;

        if reveal.intro_light && clamped_progress < LIGHT_REVEAL_HOLD {
            let warm_fg = mix_rgb(fg, Color::Rgb(255, 255, 255), 0.55);
            Self::render_static(buf, area, &GradientSpec {
                left: Color::Rgb(255, 255, 255),
                right: Color::Rgb(255, 255, 255),
                bias: 0.0,
            }, warm_fg);
            return;
        }

        if clamped_progress >= 0.999 {
            Self::render_static(buf, area, gradient, fg);
            return;
        }

        let width = area.width.max(1);
        let height = area.height.max(1);
        let width_f = width as f32;
        let height_f = height as f32;

        for row in 0..height {
            for col in 0..width {
                let x_norm = (col as f32 + 0.5) / width_f;
                let y_norm = (row as f32 + 0.5) / height_f;
                let gradient_pos = ((col as f32 / width_f) + gradient.bias).clamp(0.0, 1.0);
                let final_color = colors::mix_toward(gradient.left, gradient.right, gradient_pos);

                let coverage = reveal_coverage(
                    clamped_progress,
                    x_norm,
                    y_norm,
                    col,
                    row,
                    reveal.variant,
                );

                let softened_color = if reveal.intro_light && clamped_progress < LIGHT_REVEAL_FADE_END {
                    mix_rgb(
                        Color::Rgb(255, 255, 255),
                        final_color,
                        smoothstep(LIGHT_REVEAL_HOLD, LIGHT_REVEAL_FADE_END, clamped_progress),
                    )
                } else {
                    final_color
                };

                let mut blend = smoothstep(0.0, 1.0, coverage);
                let mut accent = accent_color(
                    reveal.variant,
                    clamped_progress,
                    x_norm,
                    y_norm,
                    col,
                    row,
                    final_color,
                );

                if reveal.intro_light {
                    let fade_factor = ((clamped_progress - LIGHT_REVEAL_HOLD)
                        / (LIGHT_REVEAL_FADE_END - LIGHT_REVEAL_HOLD))
                        .clamp(0.0, 1.0);
                    let whiten_mix = fade_factor.powf(1.1);
                    accent = mix_rgb(Color::Rgb(255, 255, 255), accent, whiten_mix);
                    blend *= fade_factor.max(0.02);
                }

                let bg = mix_rgb(accent, softened_color, blend);
                let text_tint = mix_rgb(fg, accent, (1.0 - blend) * 0.35);

                let cell = &mut buf[(area.x + col, area.y + row)];
                cell.set_symbol(" ");
                cell.set_fg(text_tint);
                cell.set_bg(bg);
            }
        }
    }
}

fn reveal_coverage(
    progress: f32,
    x: f32,
    y: f32,
    col: u16,
    row: u16,
    variant: RevealVariant,
) -> f32 {
    let base = match variant {
        RevealVariant::GlitchSweep => {
            let sweep = progress * 1.35 - x - 0.1;
            let wave = ((y * 6.0 + progress * 12.0).sin()) * 0.1;
            sweep + wave
        }
        RevealVariant::VertDrift => {
            let sweep = progress * 1.28 - x - 0.12;
            let sway = ((y * 5.0 + progress * 6.0).cos()) * 0.09;
            sweep + sway
        }
        RevealVariant::DiagonalPulse => {
            let diag = x * 0.85 + y * 0.2;
            let sweep = progress * 1.4 - diag - 0.1;
            let pulse = (((x + y) * std::f32::consts::PI * 2.0) + progress * 7.5).sin() * 0.1;
            sweep + pulse
        }
        RevealVariant::ChromaticScan => {
            let sweep = progress * 1.22 - x - 0.18 - (0.5 - y).abs() * 0.22;
            let ripple = ((progress * 10.0 + y * 5.0).cos()) * 0.07;
            sweep + ripple
        }
        RevealVariant::SparkleFade => {
            let sparkle = hash_noise(col, row);
            let flicker = ((progress * 14.0 + (x + y) * 6.0).sin()) * 0.07;
            progress * 1.55 - x - 0.22 + sparkle * 0.32 + flicker
        }
        RevealVariant::RainbowBloom => {
            let center = (x - 0.5).abs();
            let lead = (progress * 1.4 - center).max(0.0);
            let sweep = lead.powf(1.2) - 0.2;
            let ripple = ((y * 4.0 + progress * 9.0).sin()) * 0.08;
            sweep + ripple
        }
        RevealVariant::AuroraBridge => {
            let arch = (x - 0.5).abs() * 1.4 + (y - 0.5).abs() * 0.6;
            let sweep = progress * 1.35 - arch - 0.15;
            let shimmer = ((x * 12.0 + progress * 10.0).cos()) * 0.05;
            sweep + shimmer
        }
        RevealVariant::PrismRise => {
            let radial = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
            let sweep = progress * 1.45 - radial - 0.1;
            sweep + ((y * 8.0 + progress * 6.0).sin()) * 0.06
        }
        RevealVariant::NeonRoad => {
            let road = (x - 0.5).abs() * 1.1 + (y - 0.65).abs() * 0.3;
            let sweep = (progress * 1.5 - road) - 0.18;
            sweep + ((x * 6.0 + progress * 8.0).sin()) * 0.04
        }
        RevealVariant::HorizonRush => {
            let band = (x - 0.5).abs() * 0.9 + (y * 0.45);
            let horizon = (progress * 1.32 - band) - 0.12;
            horizon + ((y * 7.0 + progress * 5.0).cos()) * 0.05
        }
        RevealVariant::LightBloom => {
            let radial = ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt();
            let envelope = (progress * 1.8 - radial * 1.2).max(-0.12);
            envelope + ((x + y) * std::f32::consts::PI * 1.6).sin() * 0.05
        }
    };

    let base = if base.is_finite() { base } else { 0.0 };
    smoothstep(-0.15, 0.95, base)
}

fn accent_color(
    variant: RevealVariant,
    progress: f32,
    x: f32,
    y: f32,
    col: u16,
    row: u16,
    final_color: Color,
) -> Color {
    let seed = (x * 0.7 + y * 0.3 + progress * 0.9).fract();
    let base = gradient_multi(seed);

    match variant {
        RevealVariant::GlitchSweep => {
            let jitter = ((y * 12.0 + progress * 16.0).sin() + 1.0) * 0.25;
            mix_rgb(base, final_color, 0.3 + jitter.clamp(0.0, 0.4))
        }
        RevealVariant::VertDrift => {
            let lift = ((x * 8.0 + progress * 10.0).cos() + 1.0) * 0.2;
            mix_rgb(base, final_color, 0.35 + lift.clamp(0.0, 0.3))
        }
        RevealVariant::DiagonalPulse => {
            let pulse = (((x + y) * std::f32::consts::PI * 2.0) + progress * 6.0).sin().abs();
            let glow = mix_rgb(base, Color::Rgb(255, 255, 255), 0.4 * pulse);
            mix_rgb(glow, final_color, 0.45)
        }
        RevealVariant::ChromaticScan => {
            let scan = ((progress * 12.0 + x * 10.0).cos() + 1.0) * 0.25;
            let cool = mix_rgb(base, Color::Rgb(180, 220, 255), 0.5);
            mix_rgb(cool, final_color, 0.4 + scan.clamp(0.0, 0.3))
        }
        RevealVariant::SparkleFade => {
            let sparkle = hash_noise(col, row);
            let white = Color::Rgb(255, 255, 255);
            let glint = mix_rgb(base, white, (0.6 - progress * 0.4).max(0.0) * sparkle);
            mix_rgb(glint, final_color, 0.5)
        }
        RevealVariant::RainbowBloom => {
            let bloom = ((progress * 9.0 + x * 6.0).cos() + 1.0) * 0.35;
            let neon = mix_rgb(base, Color::Rgb(255, 180, 255), bloom * 0.3);
            mix_rgb(neon, final_color, 0.45)
        }
        RevealVariant::AuroraBridge => {
            let arch = ((y * 9.0 + progress * 8.0).sin() + 1.0) * 0.28;
            let glow = mix_rgb(base, Color::Rgb(140, 200, 255), arch * 0.4);
            mix_rgb(glow, final_color, 0.4)
        }
        RevealVariant::PrismRise => {
            let shimmer = ((x * 7.0 + y * 5.0 + progress * 6.0).sin() + 1.0) * 0.3;
            let prism = mix_rgb(base, Color::Rgb(255, 210, 120), shimmer * 0.4);
            mix_rgb(prism, final_color, 0.42)
        }
        RevealVariant::NeonRoad => {
            let strip = ((x * 12.0 + progress * 14.0).sin() + 1.0) * 0.25;
            let warm = mix_rgb(base, Color::Rgb(255, 120, 80), strip * 0.35);
            mix_rgb(warm, final_color, 0.38)
        }
        RevealVariant::HorizonRush => {
            let wash = ((y * 6.0 + progress * 7.0).cos() + 1.0) * 0.22;
            let dawn = mix_rgb(base, Color::Rgb(255, 200, 150), wash * 0.4);
            mix_rgb(dawn, final_color, 0.36)
        }
        RevealVariant::LightBloom => {
            let halo = ((progress * 6.0 + (x - 0.5).abs() * 18.0).sin() + 1.0) * 0.25;
            let glow = mix_rgb(base, Color::Rgb(255, 240, 255), halo * 0.5);
            mix_rgb(glow, final_color, 0.28)
        }
    }
}

fn hash_noise(x: u16, y: u16) -> f32 {
    let mut n = (x as u32).wrapping_mul(73856093) ^ (y as u32).wrapping_mul(19349663);
    n ^= n >> 13;
    n = n.wrapping_mul(1274126177);
    ((n >> 10) & 0xffff) as f32 / 65535.0
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 - e0).abs() < f32::EPSILON {
        return 0.0;
    }
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
