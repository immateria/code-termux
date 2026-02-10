#![allow(clippy::disallowed_methods)]

use ratatui::prelude::Color;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct GradientSpec {
    pub left: Color,
    pub right: Color,
    pub bias: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PaletteSpec {
    pub border: Color,
    pub text: Color,
    pub title: Color,
    pub footer: Color,
}

#[derive(Clone, Copy, Debug)]
pub struct RevealConfig {
    pub duration: Duration,
    pub variant: RevealVariant,
}

#[derive(Clone, Copy, Debug)]
pub enum RevealVariant {
    GlitchSweep,
    VertDrift,
    DiagonalPulse,
    ChromaticScan,
    SparkleFade,
    RainbowBloom,
    AuroraBridge,
    PrismRise,
    NeonRoad,
    HorizonRush,
    LightBloom,
}

#[derive(Clone, Copy, Debug)]
pub struct CardTheme {
    pub gradient: GradientSpec,
    pub palette: PaletteSpec,
    pub reveal: Option<RevealConfig>,
}

#[derive(Clone, Copy, Debug)]
pub struct CardThemeDefinition {
    pub name: &'static str,
    pub theme: CardTheme,
}

#[derive(Clone, Copy, Debug)]
pub struct CardPreviewSpec {
    pub name: &'static str,
    pub body: &'static [&'static str],
    pub theme: CardTheme,
}

impl CardThemeDefinition {
    pub fn preview(&self, body: &'static [&'static str]) -> CardPreviewSpec {
        CardPreviewSpec {
            name: self.name,
            body,
            theme: self.theme,
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

fn gradient(left: (u8, u8, u8), right: (u8, u8, u8), bias: f32) -> GradientSpec {
    GradientSpec {
        left: rgb(left.0, left.1, left.2),
        right: rgb(right.0, right.1, right.2),
        bias,
    }
}

fn palette(
    border: (u8, u8, u8),
    text: (u8, u8, u8),
    title: (u8, u8, u8),
    footer: (u8, u8, u8),
) -> PaletteSpec {
    PaletteSpec {
        border: rgb(border.0, border.1, border.2),
        text: rgb(text.0, text.1, text.2),
        title: rgb(title.0, title.1, title.2),
        footer: rgb(footer.0, footer.1, footer.2),
    }
}

fn reveal(duration_ms: u64, variant: RevealVariant) -> Option<RevealConfig> {
    Some(RevealConfig {
        duration: Duration::from_millis(duration_ms),
        variant,
    })
}

pub fn search_dark_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Search Dark",
        theme: CardTheme {
            gradient: gradient((66, 51, 0), (109, 95, 48), -0.05),
            palette: palette((174, 144, 50), (255, 239, 210), (255, 247, 224), (224, 203, 120)),
            reveal: None,
        },
    }
}

pub fn auto_drive_dark_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Auto Drive Dark",
        theme: CardTheme {
            gradient: gradient((2, 75, 128), (128, 81, 3), -0.05),
            palette: palette((140, 110, 200), (236, 228, 255), (246, 238, 255), (188, 170, 228)),
            reveal: reveal(1440, RevealVariant::RainbowBloom),
        },
    }
}

pub fn agent_orange_dark_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Agent Orange Dark",
        theme: CardTheme {
            gradient: gradient((39, 18, 2), (114, 62, 22), -0.05),
            palette: palette((180, 100, 40), (252, 235, 220), (255, 242, 226), (210, 168, 132)),
            reveal: None,
        },
    }
}

pub fn browser_dark_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Browser Dark",
        theme: CardTheme {
            gradient: gradient((1, 13, 33), (1, 23, 60), -0.05),
            palette: palette((110, 150, 210), (230, 242, 255), (244, 248, 255), (180, 205, 236)),
            reveal: None,
        },
    }
}

pub fn agent_green_dark_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Agent Green Dark",
        theme: CardTheme {
            gradient: gradient((6, 33, 10), (0, 90, 13), -0.05),
            palette: palette((120, 180, 130), (226, 250, 230), (240, 252, 240), (180, 220, 188)),
            reveal: None,
        },
    }
}

pub fn search_light_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Search Light",
        theme: CardTheme {
            gradient: gradient((254, 250, 237), (255, 254, 250), -0.05),
            palette: palette((215, 196, 140), (134, 98, 30), (97, 75, 24), (167, 134, 60)),
            reveal: None,
        },
    }
}

pub fn auto_drive_light_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Auto Drive Light",
        theme: CardTheme {
            gradient: gradient((232, 246, 255), (255, 241, 226), -0.05),
            palette: palette((170, 195, 222), (52, 75, 109), (30, 54, 85), (92, 121, 153)),
            reveal: reveal(1440, RevealVariant::LightBloom),
        },
    }
}

pub fn agent_orange_light_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Agent Orange Light",
        theme: CardTheme {
            gradient: gradient((244, 211, 193), (255, 243, 237), -0.05),
            palette: palette((216, 166, 132), (142, 76, 44), (110, 55, 28), (184, 111, 77)),
            reveal: None,
        },
    }
}

pub fn browser_light_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Browser Light",
        theme: CardTheme {
            gradient: gradient((227, 234, 246), (244, 247, 253), -0.05),
            palette: palette((160, 185, 220), (40, 66, 110), (26, 46, 88), (82, 111, 150)),
            reveal: None,
        },
    }
}

pub fn agent_green_light_theme() -> CardThemeDefinition {
    CardThemeDefinition {
        name: "Agent Green Light",
        theme: CardTheme {
            gradient: gradient((236, 253, 238), (249, 255, 250), -0.05),
            palette: palette((156, 206, 168), (40, 90, 54), (30, 70, 44), (86, 128, 95)),
            reveal: None,
        },
    }
}

pub fn dark_theme_catalog() -> Vec<CardThemeDefinition> {
    vec![
        search_dark_theme(),
        auto_drive_dark_theme(),
        agent_read_only_dark_theme(),
        browser_dark_theme(),
        agent_write_dark_theme(),
    ]
}

pub fn light_theme_catalog() -> Vec<CardThemeDefinition> {
    vec![
        search_light_theme(),
        auto_drive_light_theme(),
        agent_read_only_light_theme(),
        browser_light_theme(),
        agent_write_light_theme(),
    ]
}

pub fn theme_catalog() -> Vec<CardThemeDefinition> {
    let mut themes = dark_theme_catalog();
    themes.extend(light_theme_catalog());
    themes
}

pub fn auto_drive_theme_catalog() -> Vec<CardThemeDefinition> {
    vec![auto_drive_dark_theme(), auto_drive_light_theme()]
}

pub fn agent_read_only_dark_theme() -> CardThemeDefinition {
    agent_green_dark_theme()
}

pub fn agent_read_only_light_theme() -> CardThemeDefinition {
    agent_green_light_theme()
}

pub fn agent_write_dark_theme() -> CardThemeDefinition {
    agent_orange_dark_theme()
}

pub fn agent_write_light_theme() -> CardThemeDefinition {
    agent_orange_light_theme()
}
