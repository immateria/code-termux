use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::prelude::Widget;
use ratatui::widgets::{Block, Borders, Clear};

use crate::colors;
use crate::util::buffer::fill_rect;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PanelFrameStyle {
    pub(crate) title_alignment: Alignment,
    pub(crate) title_style: Style,
    pub(crate) border_style: Style,
    pub(crate) background_style: Style,
    pub(crate) content_margin: Margin,
    pub(crate) clear_background: bool,
    pub(crate) fill_inner: bool,
}

impl PanelFrameStyle {
    pub(crate) fn overlay() -> Self {
        Self {
            title_alignment: Alignment::Left,
            title_style: Style::default()
                .fg(colors::text())
                .add_modifier(Modifier::BOLD),
            border_style: Style::default()
                .fg(colors::border())
                .bg(colors::background()),
            background_style: Style::default()
                .bg(colors::background())
                .fg(colors::text()),
            content_margin: Margin::new(0, 0),
            clear_background: true,
            fill_inner: true,
        }
    }

    pub(crate) fn bottom_pane() -> Self {
        Self {
            title_alignment: Alignment::Center,
            ..Self::overlay()
        }
    }

    pub(crate) fn with_margin(mut self, margin: Margin) -> Self {
        self.content_margin = margin;
        self
    }

}

pub(crate) fn render_panel<F>(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    style: PanelFrameStyle,
    mut render_body: F,
)
where
    F: FnMut(Rect, &mut Buffer),
{
    if area.width == 0 || area.height == 0 {
        return;
    }

    if style.clear_background {
        Clear.render(area, buf);
    }

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(style.border_style)
        .style(style.background_style)
        .title_alignment(style.title_alignment);

    if !title.is_empty() {
        let title_span = Span::styled(format!(" {title} "), style.title_style);
        block = block.title(Line::from(vec![title_span]));
    }

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if style.fill_inner {
        fill_rect(buf, inner, Some(' '), style.background_style);
    }

    let content_area = inner.inner(style.content_margin);
    if content_area.width == 0 || content_area.height == 0 {
        return;
    }

    render_body(content_area, buf);
}
