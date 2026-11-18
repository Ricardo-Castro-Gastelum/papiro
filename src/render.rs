use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, PromptKind};
use crate::editor::{Editor, VisualRow, display_range};
use crate::syntax;

const SAVE_HINT: &str = "Ctrl+S · guardar   Ctrl+V · vista   Ctrl+F · buscar   Ctrl+Z · deshacer   Ctrl+Y · rehacer   Ctrl+Q · salir ";
const VIEW_HINT: &str = "Ctrl+V · editar   Ctrl+F · buscar   Ctrl+Q · salir ";
const COMPACT_SAVE_HINT: &str = "^S guardar ^V vista ^F buscar ^Z deshacer ^Y rehacer ^Q salir ";
const COMPACT_VIEW_HINT: &str = "^V editar ^F buscar ^Q salir ";
const MINIMAL_SAVE_HINT: &str = "^S ^V ^F ^Z ^Y ^Q ";
const MINIMAL_VIEW_HINT: &str = "^V ^F ^Q ";
const VIM_HINT: &str = "i insertar   Esc normal   :w guardar   :q salir   / buscar   Ctrl+V vista ";
const VIM_INSERT_HINT: &str = "Esc normal   Ctrl+S guardar   Ctrl+V vista   Ctrl+Q salir ";
const VIM_VIEW_HINT: &str = "j/k desplazar   Esc fuente   Ctrl+V editar ";
const VIM_COMPACT_HINT: &str = "i ins Esc norm :w g :q s / b ^V vista ";
const VIM_MINIMAL_HINT: &str = "^V ^Q ";
const PROMPT_LABEL: &str = " Guardar como: ";
const SEARCH_LABEL: &str = " Buscar: ";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let [text_area, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    app.set_text_area(text_area);

    let source_rows = if app.is_view_mode() {
        app.ensure_view(text_area.width, usize::from(text_area.height));
        draw_view(frame, text_area, app);
        None
    } else {
        app.editor
            .ensure_visible(text_area.width as usize, text_area.height as usize);
        let rows = app.editor.rows();
        draw_source(frame, text_area, app, &rows);
        Some(rows)
    };

    draw_footer(frame, footer_area, app);

    if let Some(input) = app.prompt_input() {
        let label = prompt_label(app.prompt_kind());
        if let Some(position) = prompt_cursor(footer_area, label, input) {
            frame.set_cursor_position(position);
        }
    } else if let Some(rows) = &source_rows
        && let Some(position) = cursor_position(&app.editor, rows, text_area)
    {
        frame.set_cursor_position(position);
    }
}

fn draw_source(frame: &mut Frame, area: Rect, app: &App, rows: &[VisualRow]) {
    let start = app.editor.scroll_row().min(rows.len());
    let end = (start + usize::from(area.height)).min(rows.len());

    let fences = syntax::fence_states(app.editor.lines());
    let lines = rows[start..end]
        .iter()
        .map(|row| source_row(app, row, fences.get(row.line).copied().unwrap_or(false)))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), area);
}

fn source_row(app: &App, row: &VisualRow, in_fence: bool) -> Line<'static> {
    let line = &app.editor.lines()[row.line];
    let theme = app.theme();
    let mut pieces = Vec::new();

    for segment in syntax::highlight(line, in_fence, theme) {
        let start = segment.start.max(row.start);
        let end = segment.end.min(row.end);
        if start < end {
            pieces.push((start, end, segment.style));
        }
    }

    if let Some((match_line, match_start, match_end)) = app.current_match()
        && match_line == row.line
    {
        let start = match_start.max(row.start);
        let end = match_end.min(row.end);
        if start < end {
            pieces = mark_match(pieces, start, end, theme.search());
        }
    }

    let spans = pieces
        .into_iter()
        .map(|(start, end, style)| Span::styled(display_range(line, start, end), style))
        .collect::<Vec<_>>();

    Line::from(spans)
}

fn mark_match(
    pieces: Vec<(usize, usize, Style)>,
    start: usize,
    end: usize,
    highlight: Style,
) -> Vec<(usize, usize, Style)> {
    let mut result = Vec::new();

    for (piece_start, piece_end, style) in pieces {
        if piece_end <= start || piece_start >= end {
            result.push((piece_start, piece_end, style));
            continue;
        }

        if piece_start < start {
            result.push((piece_start, start, style));
        }

        let marked_start = piece_start.max(start);
        let marked_end = piece_end.min(end);
        result.push((marked_start, marked_end, style.patch(highlight)));

        if piece_end > end {
            result.push((end, piece_end, style));
        }
    }

    result
}

fn draw_view(frame: &mut Frame, area: Rect, app: &App) {
    let lines = app.view_lines();
    let start = app.view_scroll().min(lines.len());
    let end = (start + usize::from(area.height)).min(lines.len());
    let visible = lines[start..end].to_vec();

    frame.render_widget(Paragraph::new(visible), area);
}

fn cursor_position(editor: &Editor, rows: &[VisualRow], area: Rect) -> Option<(u16, u16)> {
    let row = u16::try_from(editor.cursor_row(rows).checked_sub(editor.scroll_row())?).ok()?;
    let column = u16::try_from(editor.cursor_row_cell(rows)).ok()?;

    if row >= area.height || column >= area.width {
        return None;
    }

    Some((area.x.saturating_add(column), area.y.saturating_add(row)))
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let theme = app.theme();
    let dim = theme.dim();

    let blank = " ".repeat(usize::from(area.width));
    frame.render_widget(Paragraph::new(blank), area);

    if let Some(input) = app.prompt_input() {
        let label = prompt_label(app.prompt_kind());
        let mut spans = vec![
            Span::styled(label, theme.prompt()),
            Span::raw(safe_text(input)),
        ];

        if let Some(status) = app.search_status() {
            spans.push(Span::styled(format!("   {status}"), dim));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    if let Some(message) = app.message() {
        let style = if app.message_is_error() {
            theme.error()
        } else {
            Style::default()
        };
        let line = Line::from(Span::styled(format!(" {}", safe_text(message)), style));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let mut name_spans = Vec::new();
    if app.is_view_mode() {
        name_spans.push(Span::styled(" Vista · ", theme.mode()));
    } else {
        name_spans.push(Span::styled(" ", dim));
    }
    let label = app.mode_label();
    if !label.is_empty() {
        name_spans.push(Span::styled(format!(" {label} "), theme.mode()));
    }
    name_spans.push(Span::styled(safe_text(&app.editor.display_name()), dim));
    if app.editor.is_modified() {
        name_spans.push(Span::styled(" •", theme.modified()));
    }
    let name = name_spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let name_width = u16::try_from(UnicodeWidthStr::width(name.as_str())).unwrap_or(u16::MAX);

    let (full_hint, compact_hint, minimal_hint) = if app.is_vim() {
        if app.is_view_mode() {
            (VIM_VIEW_HINT, VIM_COMPACT_HINT, VIM_MINIMAL_HINT)
        } else if app.is_insert_mode() {
            (VIM_INSERT_HINT, VIM_COMPACT_HINT, VIM_MINIMAL_HINT)
        } else {
            (VIM_HINT, VIM_COMPACT_HINT, VIM_MINIMAL_HINT)
        }
    } else if app.is_view_mode() {
        (VIEW_HINT, COMPACT_VIEW_HINT, MINIMAL_VIEW_HINT)
    } else {
        (SAVE_HINT, COMPACT_SAVE_HINT, MINIMAL_SAVE_HINT)
    };

    let position = if app.is_view_mode() {
        String::new()
    } else {
        format!(
            "Ln {}, Col {}   ",
            app.editor.cursor_line() + 1,
            app.editor.cursor_column() + 1
        )
    };
    let chosen = [
        format!("{position}{full_hint}"),
        full_hint.to_owned(),
        format!("{position}{compact_hint}"),
        compact_hint.to_owned(),
        minimal_hint.to_owned(),
    ]
    .into_iter()
    .find(|hint| fits(hint, name_width, area.width))
    .unwrap_or_else(|| minimal_hint.to_owned());

    let hints = format!(" {chosen}");
    let hints_width = u16::try_from(UnicodeWidthStr::width(hints.as_str())).unwrap_or(u16::MAX);
    let name_area = Rect {
        width: area.width.saturating_sub(hints_width),
        ..area
    };

    frame.render_widget(Paragraph::new(Line::from(name_spans)), name_area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, dim))).alignment(Alignment::Right),
        area,
    );
}

fn fits(text: &str, name_width: u16, width: u16) -> bool {
    let text_width = u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX);

    text_width.saturating_add(1).saturating_add(name_width) <= width
}

fn prompt_label(kind: Option<PromptKind>) -> &'static str {
    match kind {
        Some(PromptKind::Search) => SEARCH_LABEL,
        _ => PROMPT_LABEL,
    }
}

fn prompt_cursor(area: Rect, label: &str, input: &str) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }

    let offset = UnicodeWidthStr::width(label) + UnicodeWidthStr::width(input);
    let offset = u16::try_from(offset).unwrap_or(u16::MAX);
    let last_column = area.width.saturating_sub(1);

    Some((area.x.saturating_add(offset.min(last_column)), area.y))
}

fn safe_text(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::text::Line;

    use super::{mark_match, source_row};
    use crate::app::App;
    use crate::editor::{Editor, VisualRow, display_range};
    use crate::theme::Theme;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn el_resaltado_no_cambia_el_texto_visible() {
        let app = App::with_theme(
            Editor::from_text("texto **mundo** `código`", None),
            Theme::ansi(),
        );
        let row = VisualRow {
            line: 0,
            start: 0,
            end: 24,
        };
        let line = source_row(&app, &row, false);

        assert_eq!(text(&line), "texto **mundo** `código`");
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.fg == Some(Color::Magenta))
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.fg == Some(Color::Yellow))
        );
    }

    #[test]
    fn conserva_la_expansion_de_las_pestanas_al_recortar() {
        let app = App::with_theme(Editor::from_text("a\tb", None), Theme::ansi());
        let row = VisualRow {
            line: 0,
            start: 1,
            end: 3,
        };
        let line = source_row(&app, &row, false);

        assert_eq!(text(&line), display_range("a\tb", 1, 3));
    }

    #[test]
    fn el_backend_recibe_colores_de_la_paleta() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut terminal = Terminal::new(TestBackend::new(40, 3)).expect("backend de prueba");
        let mut app = App::with_theme(Editor::from_text("# Título", None), Theme::ansi());
        terminal
            .draw(|frame| super::draw(frame, &mut app))
            .expect("dibujar");

        let buffer = terminal.backend().buffer();
        assert!(
            buffer
                .content
                .iter()
                .any(|cell| cell.style().fg == Some(Color::Cyan))
        );
    }

    #[test]
    fn la_busqueda_se_superpone_al_color_de_sintaxis() {
        let base = Style::default().fg(Color::Magenta);
        let pieces = vec![(0, 4, base), (4, 8, base)];
        let marked = mark_match(
            pieces,
            2,
            6,
            Style::default().add_modifier(Modifier::REVERSED),
        );

        assert_eq!(marked.len(), 4);
        assert!(marked[1].2.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(marked[1].2.fg, Some(Color::Magenta));
        assert!(!marked[0].2.add_modifier.contains(Modifier::REVERSED));
    }
}
