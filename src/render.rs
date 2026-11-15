//! Dibujo del modo Fuente, el indicador de nombre y la línea inferior.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, PromptKind};
use crate::editor::{Editor, VisualRow, display_range};

const SAVE_HINT: &str = "Ctrl+S · guardar   Ctrl+V · vista   Ctrl+F · buscar   Ctrl+Z · deshacer   Ctrl+Y · rehacer   Ctrl+Q · salir ";
const VIEW_HINT: &str = "Ctrl+V · editar   Ctrl+F · buscar   Ctrl+Q · salir ";
const COMPACT_SAVE_HINT: &str = "^S guardar ^V vista ^F buscar ^Z deshacer ^Y rehacer ^Q salir ";
const COMPACT_VIEW_HINT: &str = "^V editar ^F buscar ^Q salir ";
const MINIMAL_SAVE_HINT: &str = "^S ^V ^F ^Z ^Y ^Q ";
const MINIMAL_VIEW_HINT: &str = "^V ^F ^Q ";
const PROMPT_LABEL: &str = " Guardar como: ";
const SEARCH_LABEL: &str = " Buscar: ";

/// Área de texto más una línea discreta para el nombre del archivo, los
/// atajos o el nombre que se esté escribiendo. No hay bordes ni fondo propio:
/// todo hereda los colores de la terminal.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let [text_area, footer_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

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

    let lines = rows[start..end]
        .iter()
        .map(|row| source_row(app, row))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Dibuja una fila visual resaltando la coincidencia de búsqueda actual si
/// cae dentro de ella.
fn source_row(app: &App, row: &VisualRow) -> Line<'static> {
    let line = &app.editor.lines()[row.line];
    let Some((match_line, match_start, match_end)) = app.current_match() else {
        return Line::raw(display_range(line, row.start, row.end));
    };

    let start = match_start.max(row.start);
    let end = match_end.min(row.end);
    if match_line != row.line || start >= end {
        return Line::raw(display_range(line, row.start, row.end));
    }

    let highlight = Style::default().add_modifier(Modifier::REVERSED);
    let mut spans = Vec::new();

    if start > row.start {
        spans.push(Span::raw(display_range(line, row.start, start)));
    }
    spans.push(Span::styled(display_range(line, start, end), highlight));
    if end < row.end {
        spans.push(Span::raw(display_range(line, end, row.end)));
    }

    Line::from(spans)
}

/// Dibuja solo las líneas visibles del documento renderizado. El ajuste de
/// líneas ya se hizo al analizar el Markdown, así que el desplazamiento es
/// una simple rebanada.
fn draw_view(frame: &mut Frame, area: Rect, app: &App) {
    let lines = app.view_lines();
    let start = app.view_scroll().min(lines.len());
    let end = (start + usize::from(area.height)).min(lines.len());
    let visible = lines[start..end].to_vec();

    frame.render_widget(Paragraph::new(visible), area);
}

/// Posición del cursor en coordenadas de pantalla, si queda dentro del área
/// visible.
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

    let dim = Style::default().add_modifier(Modifier::DIM);

    // Limpia la línea completa para que no queden restos de la barra anterior
    // al abrir el indicador de nombre o mostrar un mensaje.
    let blank = " ".repeat(usize::from(area.width));
    frame.render_widget(Paragraph::new(blank), area);

    if let Some(input) = app.prompt_input() {
        let label = prompt_label(app.prompt_kind());
        let mut spans = vec![Span::styled(label, dim), Span::raw(safe_text(input))];

        if let Some(status) = app.search_status() {
            spans.push(Span::styled(format!("   {status}"), dim));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    if let Some(message) = app.message() {
        frame.render_widget(Paragraph::new(format!(" {}", safe_text(message))), area);
        return;
    }

    let mut name = if app.is_view_mode() {
        format!(" Vista · {}", safe_text(&app.editor.display_name()))
    } else {
        format!(" {}", safe_text(&app.editor.display_name()))
    };
    if app.editor.is_modified() {
        name.push_str(" •");
    }
    let name_width = u16::try_from(UnicodeWidthStr::width(name.as_str())).unwrap_or(u16::MAX);

    let full_hint = if app.is_view_mode() {
        VIEW_HINT
    } else {
        SAVE_HINT
    };
    let compact_hint = if app.is_view_mode() {
        COMPACT_VIEW_HINT
    } else {
        COMPACT_SAVE_HINT
    };
    let minimal_hint = if app.is_view_mode() {
        MINIMAL_VIEW_HINT
    } else {
        MINIMAL_SAVE_HINT
    };

    // Primero se intenta mostrar la posición del cursor y los atajos
    // completos; si no caben junto al nombre, se van acortando por orden.
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

    // El espacio inicial separa el nombre de los atajos.
    let hints = format!(" {chosen}");
    let hints_width = u16::try_from(UnicodeWidthStr::width(hints.as_str())).unwrap_or(u16::MAX);
    let name_area = Rect {
        width: area.width.saturating_sub(hints_width),
        ..area
    };

    frame.render_widget(Paragraph::new(name).style(dim), name_area);
    frame.render_widget(
        Paragraph::new(hints).style(dim).alignment(Alignment::Right),
        area,
    );
}

/// Indica si el texto y el nombre caben en la barra con un espacio entre ambos.
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

/// Los nombres de archivo y los mensajes pueden contener caracteres de
/// control. En la barra inferior se sustituyen para no enviar secuencias de
/// escape a la terminal.
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
