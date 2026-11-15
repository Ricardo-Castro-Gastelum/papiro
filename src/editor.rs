//! Estado del texto, edición, cursor, deshacer/rehacer y ajuste de líneas.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::Result;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::file;

/// Ancho de un tabulador al mostrarlo. La tecla Tab inserta dos espacios, pero
/// los tabuladores que ya existan en un archivo se alinean a paradas de cuatro
/// columnas.
pub const TAB_DISPLAY_WIDTH: usize = 4;

/// Número máximo de estados que se conservan para deshacer.
const MAX_UNDO: usize = 200;

/// Cada línea se guarda como una cadena UTF-8. El cursor se expresa en
/// índices de grafemas, no de bytes, para que los acentos y los emojis
/// compuestos se traten como una sola unidad.
type Line = String;

/// Una fila visual del modo Fuente. Una línea lógica puede ocupar varias filas
/// cuando no cabe en el ancho de la terminal. `start` y `end` son índices de
/// grafemas dentro de esa línea.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Tipo de edición. El tecleo y los borrados seguidos se agrupan en un solo
/// paso de deshacer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Backspace,
    Delete,
}

/// Estado completo del búfer en un momento dado. Se guarda antes de cada
/// edición para poder restaurarlo después.
#[derive(Clone)]
struct Snapshot {
    lines: Vec<Line>,
    cursor_line: usize,
    cursor_column: usize,
}

pub struct Editor {
    lines: Vec<Line>,
    path: Option<PathBuf>,
    cursor_line: usize,
    cursor_column: usize,
    scroll_row: usize,
    viewport_width: usize,
    viewport_height: usize,
    /// Cambios pendientes desde la última carga o guardado.
    modified: bool,
    /// Copia del texto tal como está en disco. Sirve para saber si deshacer
    /// volvió exactamente al estado guardado.
    saved: Vec<Line>,
    undo: VecDeque<Snapshot>,
    redo: VecDeque<Snapshot>,
    /// Tipo de edición de la racha actual. Mientras no se mueva el cursor ni
    /// se guarde, las pulsaciones del mismo tipo comparten un paso de
    /// deshacer.
    coalesce: Option<EditKind>,
}

impl Editor {
    /// Crea un búfer vacío o carga el archivo indicado. Si la ruta no existe,
    /// el búfer queda asociado a ella y se creará al guardar.
    pub fn open(path: Option<PathBuf>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::from_text("", None));
        };

        let text = file::read_file(&path)?.unwrap_or_default();
        Ok(Self::from_text(&text, Some(path)))
    }

    pub fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        let lines: Vec<Line> = text.split('\n').map(str::to_owned).collect();

        Self {
            saved: lines.clone(),
            lines,
            path,
            cursor_line: 0,
            cursor_column: 0,
            scroll_row: 0,
            viewport_width: 0,
            viewport_height: 0,
            modified: false,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            coalesce: None,
        }
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    /// Texto completo tal como se guardará en disco.
    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    /// Nombre breve para la barra inferior. Si no hay ruta, se muestra un
    /// texto genérico en lugar de inventar un nombre de archivo.
    pub fn display_name(&self) -> String {
        match &self.path {
            Some(path) => match path.file_name() {
                Some(name) if !name.is_empty() => name.to_string_lossy().into_owned(),
                _ => path.display().to_string(),
            },
            None => "sin título".to_owned(),
        }
    }

    pub fn cursor_line(&self) -> usize {
        self.cursor_line
    }

    pub fn cursor_column(&self) -> usize {
        self.cursor_column
    }

    /// Coloca el cursor en una posición concreta. Se usa al saltar a una
    /// coincidencia de búsqueda; si la línea no existe, no hace nada.
    pub fn set_cursor(&mut self, line: usize, column: usize) {
        if line >= self.lines.len() {
            return;
        }

        self.cursor_line = line;
        self.cursor_column = column.min(grapheme_count(&self.lines[line]));
        self.coalesce = None;
    }

    /// Indica si hay cambios pendientes de guardar.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Marca el contenido actual como guardado.
    pub fn mark_saved(&mut self) {
        self.modified = false;
        self.saved.clone_from(&self.lines);
        self.coalesce = None;
    }

    pub fn scroll_row(&self) -> usize {
        self.scroll_row
    }

    /// Columna visual del cursor, contando tabuladores y caracteres de doble
    /// ancho. Es distinta de `cursor_column`, que cuenta grafemas.
    pub fn cursor_cell(&self) -> usize {
        cell_width(&self.lines[self.cursor_line], self.cursor_column)
    }

    /// Filas visuales con el ajuste de líneas del ancho actual. El resultado
    /// siempre tiene al menos una fila por cada línea lógica.
    pub fn rows(&self) -> Vec<VisualRow> {
        wrap_rows(&self.lines, self.wrap_width())
    }

    /// Fila visual que ocupa el cursor dentro del ajuste actual.
    pub fn cursor_row(&self, rows: &[VisualRow]) -> usize {
        row_index(rows, self.cursor_line, self.cursor_column)
    }

    /// Columna del cursor dentro de su fila visual.
    pub fn cursor_row_cell(&self, rows: &[VisualRow]) -> usize {
        let Some(row) = rows.get(self.cursor_row(rows)) else {
            return self.cursor_cell();
        };

        self.cursor_cell()
            .saturating_sub(cell_width(&self.lines[row.line], row.start))
    }

    /// Corta la agrupación de deshacer. Se usa al cambiar de modo para que el
    /// tecleo anterior no se mezcle con el siguiente.
    pub fn break_undo_group(&mut self) {
        self.coalesce = None;
    }

    pub fn move_up(&mut self) {
        let rows = self.rows();
        let current = self.cursor_row(&rows);

        if current > 0 {
            self.move_to_row(&rows, current - 1);
        }
        self.coalesce = None;
    }

    pub fn move_down(&mut self) {
        let rows = self.rows();
        let current = self.cursor_row(&rows);

        if current + 1 < rows.len() {
            self.move_to_row(&rows, current + 1);
        }
        self.coalesce = None;
    }

    pub fn move_left(&mut self) {
        self.cursor_column = self.cursor_column.saturating_sub(1);
        self.coalesce = None;
    }

    pub fn move_right(&mut self) {
        if self.cursor_column < grapheme_count(&self.lines[self.cursor_line]) {
            self.cursor_column += 1;
        }
        self.coalesce = None;
    }

    pub fn move_home(&mut self) {
        self.cursor_column = 0;
        self.coalesce = None;
    }

    pub fn move_end(&mut self) {
        self.cursor_column = grapheme_count(&self.lines[self.cursor_line]);
        self.coalesce = None;
    }

    /// Avanza hasta el comienzo de la palabra siguiente. Los signos de
    /// puntuación y los espacios se saltan como separadores.
    pub fn move_word_right(&mut self) {
        let graphemes = graphemes(&self.lines[self.cursor_line]);
        let mut index = self.cursor_column;

        while index < graphemes.len() && is_word_grapheme(graphemes[index]) {
            index += 1;
        }
        while index < graphemes.len() && !is_word_grapheme(graphemes[index]) {
            index += 1;
        }

        self.cursor_column = index;
        self.coalesce = None;
    }

    /// Retrocede hasta el comienzo de la palabra anterior.
    pub fn move_word_left(&mut self) {
        let graphemes = graphemes(&self.lines[self.cursor_line]);
        let mut index = self.cursor_column;

        while index > 0 && !is_word_grapheme(graphemes[index - 1]) {
            index -= 1;
        }
        while index > 0 && is_word_grapheme(graphemes[index - 1]) {
            index -= 1;
        }

        self.cursor_column = index;
        self.coalesce = None;
    }

    pub fn page_up(&mut self) {
        let rows = self.rows();
        let current = self.cursor_row(&rows);
        let step = self.page_step();

        self.move_to_row(&rows, current.saturating_sub(step));
        self.coalesce = None;
    }

    pub fn page_down(&mut self) {
        let rows = self.rows();
        let current = self.cursor_row(&rows);
        let step = self.page_step();
        let target = (current + step).min(rows.len() - 1);

        self.move_to_row(&rows, target);
        self.coalesce = None;
    }

    fn page_step(&self) -> usize {
        self.viewport_height.saturating_sub(1).max(1)
    }

    /// Mueve el cursor a otra fila visual conservando, en lo posible, la
    /// columna que ocupaba en la fila anterior.
    fn move_to_row(&mut self, rows: &[VisualRow], target: usize) {
        let current = self.cursor_row(rows);
        let (Some(current_row), Some(row)) = (rows.get(current), rows.get(target)) else {
            return;
        };

        let desired = self
            .cursor_cell()
            .saturating_sub(cell_width(&self.lines[current_row.line], current_row.start));

        self.cursor_line = row.line;
        self.cursor_column = column_at_cell(&self.lines[row.line], row.start, row.end, desired);
    }

    /// Inserta un solo carácter en la posición del cursor. Los caracteres de
    /// control se ignoran: nunca deben llegar al búfer desde las teclas.
    pub fn insert_char(&mut self, character: char) {
        if character == '\n' {
            self.insert_newline();
        } else if !character.is_control() {
            self.insert_in_line(character.encode_utf8(&mut [0; 4]));
        }
    }

    pub fn insert_newline(&mut self) {
        self.split_line();
    }

    /// Tab inserta dos espacios, como pide la interfaz.
    pub fn insert_tab(&mut self) {
        self.insert_in_line("  ");
    }

    fn insert_in_line(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        self.begin_edit(Some(EditKind::Typing));

        let column;
        {
            let line = &mut self.lines[self.cursor_line];
            let offset = grapheme_offset(line, self.cursor_column);
            line.insert_str(offset, text);
            column = grapheme_count(&line[..offset + text.len()]);
        }
        self.cursor_column = column;
        self.modified = true;
    }

    fn split_line(&mut self) {
        self.begin_edit(None);

        let rest;
        {
            let line = &mut self.lines[self.cursor_line];
            let offset = grapheme_offset(line, self.cursor_column);
            rest = line.split_off(offset);
        }

        self.lines.insert(self.cursor_line + 1, rest);
        self.cursor_line += 1;
        self.cursor_column = 0;
        self.modified = true;
    }

    /// Borra el grafema anterior. Al principio de una línea, une esa línea con
    /// la anterior.
    pub fn backspace(&mut self) {
        if self.cursor_column > 0 {
            self.begin_edit(Some(EditKind::Backspace));

            let column;
            {
                let line = &mut self.lines[self.cursor_line];
                let start = grapheme_offset(line, self.cursor_column - 1);
                let end = grapheme_offset(line, self.cursor_column);
                line.replace_range(start..end, "");
                column = self.cursor_column - 1;
            }
            self.cursor_column = column;
            self.modified = true;
        } else if self.cursor_line > 0 {
            self.begin_edit(Some(EditKind::Backspace));

            let current = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;

            let previous = &mut self.lines[self.cursor_line];
            let previous_length = grapheme_count(previous);
            previous.push_str(&current);
            self.cursor_column = previous_length;
            self.modified = true;
        }
    }

    /// Borra el grafema bajo el cursor. Al final de una línea, une la
    /// siguiente con la actual.
    pub fn delete(&mut self) {
        if self.cursor_column < grapheme_count(&self.lines[self.cursor_line]) {
            self.begin_edit(Some(EditKind::Delete));

            let line = &mut self.lines[self.cursor_line];
            let start = grapheme_offset(line, self.cursor_column);
            let end = grapheme_offset(line, self.cursor_column + 1);
            line.replace_range(start..end, "");
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            self.begin_edit(Some(EditKind::Delete));

            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
        }
    }

    /// Deshace la última edición o racha de ediciones.
    pub fn undo(&mut self) {
        let Some(snapshot) = self.undo.pop_back() else {
            return;
        };

        self.redo.push_back(self.snapshot());
        self.restore(snapshot);
        self.coalesce = None;
        self.modified = self.lines != self.saved;
    }

    /// Rehace la última edición deshecha.
    pub fn redo(&mut self) {
        let Some(snapshot) = self.redo.pop_back() else {
            return;
        };

        if self.undo.len() == MAX_UNDO {
            self.undo.pop_front();
        }
        self.undo.push_back(self.snapshot());
        self.restore(snapshot);
        self.coalesce = None;
        self.modified = self.lines != self.saved;
    }

    /// Guarda el estado actual antes de una edición. Si la edición continúa
    /// una racha del mismo tipo, no se duplica el punto de deshacer.
    fn begin_edit(&mut self, kind: Option<EditKind>) {
        let merge = kind.is_some() && self.coalesce == kind;

        if !merge {
            if self.undo.len() == MAX_UNDO {
                self.undo.pop_front();
            }
            self.undo.push_back(self.snapshot());
        }

        self.redo.clear();
        self.coalesce = kind;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor_line: self.cursor_line,
            cursor_column: self.cursor_column,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.lines = snapshot.lines;
        self.cursor_line = snapshot.cursor_line;
        self.cursor_column = snapshot.cursor_column;
    }

    /// Ancho con el que se ajustan las líneas. Antes del primer fotograma no
    /// se conoce el ancho de la terminal y se muestra una fila por línea.
    fn wrap_width(&self) -> usize {
        if self.viewport_width == 0 {
            usize::MAX
        } else {
            self.viewport_width
        }
    }

    /// Ajusta el desplazamiento para que el cursor quede dentro del área de
    /// texto. Se llama en cada fotograma, después de conocer el tamaño real.
    pub fn ensure_visible(&mut self, width: usize, height: usize) {
        self.viewport_width = width;
        self.viewport_height = height;

        if width == 0 || height == 0 {
            return;
        }

        let rows = self.rows();
        let cursor = self.cursor_row(&rows);

        if cursor < self.scroll_row {
            self.scroll_row = cursor;
        } else if cursor >= self.scroll_row + height {
            self.scroll_row = cursor + 1 - height;
        }

        self.scroll_row = self.scroll_row.min(rows.len().saturating_sub(height));
    }
}

/// Divide las líneas lógicas en filas visuales que caben en `width` columnas.
/// Los caracteres de doble ancho que no caben al final de una fila pasan
/// enteros a la siguiente para no partirlos.
fn wrap_rows(lines: &[Line], width: usize) -> Vec<VisualRow> {
    let width = width.max(1);
    let mut rows = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let graphemes = graphemes(line);
        if graphemes.is_empty() {
            rows.push(VisualRow {
                line: line_index,
                start: 0,
                end: 0,
            });
            continue;
        }

        let mut start = 0;
        let mut start_cell = 0;
        let mut cell = 0;

        for (index, grapheme) in graphemes.iter().enumerate() {
            let grapheme_width = grapheme_width(grapheme, cell);
            if index > start && cell + grapheme_width - start_cell > width {
                rows.push(VisualRow {
                    line: line_index,
                    start,
                    end: index,
                });
                start = index;
                start_cell = cell;
            }
            cell += grapheme_width;
        }

        rows.push(VisualRow {
            line: line_index,
            start,
            end: graphemes.len(),
        });

        // Si la línea ocupa el ancho exacto, el cursor al final necesita una
        // fila más donde dibujarse; de lo contrario quedaría fuera de la
        // pantalla.
        if cell > 0 && cell % width == 0 {
            rows.push(VisualRow {
                line: line_index,
                start: graphemes.len(),
                end: graphemes.len(),
            });
        }
    }

    rows
}

/// Índice de la fila visual que contiene la columna indicada. Si la columna
/// está justo en un corte, se elige la fila siguiente, que es donde debe
/// dibujarse el cursor.
fn row_index(rows: &[VisualRow], line: usize, column: usize) -> usize {
    let mut fallback = None;

    for (index, row) in rows.iter().enumerate() {
        if row.line != line {
            continue;
        }

        fallback = Some(index);
        if column < row.end || row.start == row.end {
            return index;
        }
    }

    fallback.unwrap_or(0)
}

/// Índice de grafema más cercano a `offset` columnas desde el comienzo de la
/// fila. Si el desplazamiento supera el ancho de la fila, devuelve su final.
fn column_at_cell(line: &str, start: usize, end: usize, offset: usize) -> usize {
    let graphemes = graphemes(line);
    let start_cell = cell_width(line, start);
    let mut column = start_cell;

    for (index, grapheme) in graphemes.iter().enumerate().take(end).skip(start) {
        if column - start_cell >= offset {
            return index;
        }
        column += grapheme_width(grapheme, column);
    }

    end
}

fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

fn graphemes(text: &str) -> Vec<&str> {
    text.graphemes(true).collect()
}

/// Un grafema se considera parte de una palabra si empieza por una letra, un
/// número o un guion bajo. Los emojis y la puntuación actúan como separadores.
fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
}

/// Desplazamiento en bytes del grafema indicado. Si el índice está al final,
/// devuelve la longitud de la cadena.
fn grapheme_offset(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .nth(index)
        .map_or(text.len(), |(offset, _)| offset)
}

/// Calcula la anchura visual de los primeros `limit` grafemas. Los tabuladores
/// avanzan hasta la siguiente parada y los caracteres de control se sustituyen
/// por un símbolo visible de una columna.
fn cell_width(text: &str, limit: usize) -> usize {
    text.graphemes(true)
        .take(limit)
        .fold(0, |column, grapheme| {
            column + grapheme_width(grapheme, column)
        })
}

fn grapheme_width(grapheme: &str, column: usize) -> usize {
    if grapheme == "\t" {
        TAB_DISPLAY_WIDTH - (column % TAB_DISPLAY_WIDTH)
    } else if grapheme.chars().any(char::is_control) {
        1
    } else {
        UnicodeWidthStr::width(grapheme)
    }
}

/// Convierte una línea interna en el texto que se dibuja, o solo el fragmento
/// de `start..end` cuando la línea está ajustada. Los tabuladores se expanden
/// con paradas fijas y los caracteres de control no llegan nunca a la
/// terminal: se muestran como `�` para evitar secuencias de escape ajenas.
///
/// Los tabuladores se alinean según su columna real dentro de la línea, no
/// dentro del fragmento, para que el ajuste no rompa la sangría.
pub fn display_range(line: &str, start: usize, end: usize) -> String {
    let mut rendered = String::new();
    let mut column = 0;

    for (index, grapheme) in line.graphemes(true).enumerate() {
        if index >= end {
            break;
        }
        if index < start {
            column += grapheme_width(grapheme, column);
            continue;
        }

        if grapheme == "\t" {
            let spaces = TAB_DISPLAY_WIDTH - (column % TAB_DISPLAY_WIDTH);
            rendered.push_str(&" ".repeat(spaces));
            column += spaces;
        } else if grapheme.chars().any(char::is_control) {
            rendered.push('\u{FFFD}');
            column += 1;
        } else {
            rendered.push_str(grapheme);
            column += UnicodeWidthStr::width(grapheme);
        }
    }

    rendered
}

#[cfg(test)]
mod tests {
    use super::{Editor, VisualRow, cell_width, display_range, grapheme_count, wrap_rows};

    #[test]
    fn mueve_el_cursor_por_palabras() {
        let mut editor = Editor::from_text("hola, mundo feliz", None);

        editor.move_word_right();
        assert_eq!(editor.cursor_column, 6);

        editor.move_word_right();
        assert_eq!(editor.cursor_column, 12);

        editor.move_word_left();
        assert_eq!(editor.cursor_column, 6);

        editor.move_word_left();
        assert_eq!(editor.cursor_column, 0);
    }

    #[test]
    fn conserva_la_linea_vacia_final() {
        let editor = Editor::from_text("uno\ndos\n", None);

        assert_eq!(editor.lines().len(), 3);
        assert!(editor.lines()[2].is_empty());
    }

    #[test]
    fn mueve_el_cursor_por_grafemas() {
        let mut editor = Editor::from_text("año 👋🏽", None);

        editor.move_end();
        assert_eq!(editor.cursor_column, grapheme_count(&editor.lines()[0]));

        editor.move_left();
        assert_eq!(editor.cursor_column, grapheme_count(&editor.lines()[0]) - 1);
    }

    #[test]
    fn inserta_texto_y_saltos_de_linea() {
        let mut editor = Editor::from_text("", None);

        for character in "uno".chars() {
            editor.insert_char(character);
        }
        assert_eq!(editor.content(), "uno");

        editor.insert_newline();
        for character in "dos".chars() {
            editor.insert_char(character);
        }
        assert_eq!(editor.content(), "uno\ndos");
        assert_eq!(editor.cursor_line, 1);
        assert_eq!(editor.cursor_column, 3);

        editor.insert_newline();
        assert_eq!(editor.content(), "uno\ndos\n");
        assert_eq!(editor.cursor_line, 2);
        assert_eq!(editor.cursor_column, 0);
    }

    #[test]
    fn inserta_una_linea_en_medio_del_texto() {
        let mut editor = Editor::from_text("ac", None);

        editor.move_end();
        editor.move_left();
        editor.insert_newline();

        assert_eq!(editor.content(), "a\nc");
        assert_eq!(editor.cursor_line, 1);
        assert_eq!(editor.cursor_column, 0);
    }

    #[test]
    fn retroceso_borra_un_grafema_y_une_lineas() {
        let mut editor = Editor::from_text("año\nb", None);

        editor.move_down();
        editor.backspace();
        assert_eq!(editor.content(), "añob");
        assert_eq!(editor.cursor_line, 0);
        assert_eq!(editor.cursor_column, 3);

        editor.move_end();
        editor.backspace();
        assert_eq!(editor.content(), "año");
    }

    #[test]
    fn retroceso_borra_un_emoji_compuesto() {
        let mut editor = Editor::from_text("👋🏽", None);

        editor.move_end();
        assert_eq!(editor.cursor_column, 1);
        editor.backspace();

        assert_eq!(editor.content(), "");
        assert_eq!(editor.cursor_column, 0);
    }

    #[test]
    fn un_acento_combina_con_la_letra_anterior() {
        let mut editor = Editor::from_text("e", None);

        editor.move_end();
        editor.insert_char('\u{301}');

        assert_eq!(editor.content(), "e\u{301}");
        assert_eq!(grapheme_count(&editor.lines()[0]), 1);
        assert_eq!(editor.cursor_column, 1);

        editor.backspace();
        assert_eq!(editor.content(), "");
    }

    #[test]
    fn suprimir_borra_un_grafema_y_une_lineas() {
        let mut editor = Editor::from_text("abc", None);

        editor.delete();
        assert_eq!(editor.content(), "bc");

        let mut editor = Editor::from_text("a\nb", None);
        editor.move_end();
        editor.delete();
        assert_eq!(editor.content(), "ab");
        assert_eq!(editor.cursor_line, 0);
    }

    #[test]
    fn tab_inserta_dos_espacios() {
        let mut editor = Editor::from_text("", None);

        editor.insert_tab();

        assert_eq!(editor.content(), "  ");
        assert_eq!(editor.cursor_column, 2);
    }

    #[test]
    fn ignora_caracteres_de_control() {
        let mut editor = Editor::from_text("", None);

        editor.insert_char('\u{1b}');

        assert_eq!(editor.content(), "");
    }

    #[test]
    fn expande_tabuladores_hasta_la_siguiente_parada() {
        assert_eq!(display_range("a\tb", 0, usize::MAX), "a   b");
        assert_eq!(cell_width("a\tb", 3), 5);
    }

    #[test]
    fn calcula_la_anchura_de_emojis() {
        assert_eq!(cell_width("👋🏽a", 2), 3);
    }

    #[test]
    fn sustituye_caracteres_de_control() {
        assert_eq!(display_range("\u{1b}", 0, usize::MAX), "�");
        assert_eq!(cell_width("\u{1b}", 1), 1);
    }

    #[test]
    fn mantiene_el_cursor_visible_al_desplazarse() {
        let mut editor = Editor::from_text("uno\ndos\ntres\ncuatro\ncinco", None);

        for _ in 0..4 {
            editor.move_down();
        }
        editor.ensure_visible(10, 2);

        assert_eq!(editor.scroll_row(), 3);
    }

    #[test]
    fn acota_la_columna_al_cambiar_de_linea() {
        let mut editor = Editor::from_text("línea larga\ncorta", None);

        editor.move_end();
        editor.move_down();

        assert_eq!(editor.cursor_line(), 1);
        assert_eq!(editor.cursor_column(), grapheme_count(&editor.lines()[1]));
    }

    #[test]
    fn marca_cambios_solo_cuando_se_edita() {
        let mut editor = Editor::from_text("hola", None);
        assert!(!editor.is_modified());

        editor.mark_saved();
        assert!(!editor.is_modified());

        editor.insert_char('s');
        assert!(editor.is_modified());

        editor.mark_saved();
        assert!(!editor.is_modified());

        editor.move_end();
        assert!(!editor.is_modified());

        // Al final de la única línea, Suprimir no borra nada.
        editor.delete();
        assert!(!editor.is_modified());

        editor.backspace();
        assert!(editor.is_modified());
    }

    #[test]
    fn deshace_y_rehace_el_tecleo() {
        let mut editor = Editor::from_text("", None);

        for character in "hola".chars() {
            editor.insert_char(character);
        }
        assert_eq!(editor.content(), "hola");

        editor.undo();
        assert_eq!(editor.content(), "");

        editor.redo();
        assert_eq!(editor.content(), "hola");
    }

    #[test]
    fn el_movimiento_corta_la_agrupacion_de_deshacer() {
        let mut editor = Editor::from_text("", None);

        editor.insert_char('a');
        editor.insert_char('b');
        editor.move_home();
        editor.insert_char('x');
        assert_eq!(editor.content(), "xab");

        editor.undo();
        assert_eq!(editor.content(), "ab");

        editor.undo();
        assert_eq!(editor.content(), "");
    }

    #[test]
    fn agrupa_los_borrados_seguidos() {
        let mut editor = Editor::from_text("hola", None);
        editor.move_end();

        editor.backspace();
        editor.backspace();
        assert_eq!(editor.content(), "ho");

        editor.undo();
        assert_eq!(editor.content(), "hola");

        editor.redo();
        assert_eq!(editor.content(), "ho");
    }

    #[test]
    fn rehacer_se_pierde_al_editar_de_nuevo() {
        let mut editor = Editor::from_text("", None);

        editor.insert_char('a');
        editor.undo();
        editor.insert_char('b');
        editor.redo();

        assert_eq!(editor.content(), "b");
    }

    #[test]
    fn deshacer_restaura_la_marca_de_cambios() {
        let mut editor = Editor::from_text("hola", None);

        editor.insert_char('!');
        assert!(editor.is_modified());

        editor.undo();
        assert_eq!(editor.content(), "hola");
        assert!(!editor.is_modified());

        editor.redo();
        assert_eq!(editor.content(), "!hola");
        assert!(editor.is_modified());
    }

    #[test]
    fn guardar_corta_la_agrupacion_de_deshacer() {
        let mut editor = Editor::from_text("", None);

        editor.insert_char('a');
        editor.mark_saved();
        editor.insert_char('b');

        editor.undo();
        assert_eq!(editor.content(), "a");
        assert!(!editor.is_modified());

        editor.undo();
        assert_eq!(editor.content(), "");
        assert!(editor.is_modified());
    }

    #[test]
    fn deshacer_restaura_el_cursor() {
        let mut editor = Editor::from_text("hola", None);

        editor.move_end();
        editor.insert_char('!');
        assert_eq!(editor.cursor_column, 5);

        editor.undo();

        assert_eq!(editor.cursor_column, 4);
    }

    #[test]
    fn ajusta_lineas_largas_en_varias_filas() {
        let rows = wrap_rows(&["1234567890".to_owned()], 4);

        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            VisualRow {
                line: 0,
                start: 0,
                end: 4,
            }
        );
        assert_eq!(
            rows[2],
            VisualRow {
                line: 0,
                start: 8,
                end: 10,
            }
        );
    }

    #[test]
    fn reserva_una_fila_para_el_final_de_una_linea_completa() {
        let rows = wrap_rows(&["1234".to_owned()], 4);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].start, 4);
        assert_eq!(rows[1].end, 4);
    }

    #[test]
    fn deja_una_fila_por_cada_linea_vacia() {
        let rows = wrap_rows(&["uno".to_owned(), String::new()], 10);

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].line, 1);
        assert_eq!(rows[1].start, rows[1].end);
    }

    #[test]
    fn mueve_el_cursor_entre_filas_ajustadas() {
        let mut editor = Editor::from_text("abcdefgh", None);
        editor.ensure_visible(4, 10);

        editor.move_down();
        assert_eq!(editor.cursor_line(), 0);
        assert_eq!(editor.cursor_column(), 4);

        // La línea ocupa el ancho exacto, así que el final del texto tiene su
        // propia fila visual.
        editor.move_down();
        assert_eq!(editor.cursor_column(), 8);

        editor.move_down();
        assert_eq!(editor.cursor_column(), 8);

        editor.move_up();
        assert_eq!(editor.cursor_column(), 4);
    }

    #[test]
    fn conserva_la_columna_visual_al_bajar_entre_lineas() {
        let mut editor = Editor::from_text("abcdefghi\nxy", None);
        editor.ensure_visible(4, 10);

        editor.move_end();
        editor.move_down();
        assert_eq!(editor.cursor_line(), 1);
        assert_eq!(editor.cursor_column(), 1);

        editor.move_up();
        assert_eq!(editor.cursor_line(), 0);
        assert_eq!(editor.cursor_column(), 9);
    }

    #[test]
    fn coloca_el_cursor_en_la_fila_ajustada() {
        let mut editor = Editor::from_text("abcdefghi", None);
        editor.ensure_visible(4, 5);
        editor.move_end();

        let rows = editor.rows();
        assert_eq!(editor.cursor_row(&rows), 2);
        assert_eq!(editor.cursor_row_cell(&rows), 1);
    }

    #[test]
    fn mantiene_visible_la_fila_del_cursor_ajustado() {
        let mut editor = Editor::from_text(&"x".repeat(30), None);
        editor.ensure_visible(5, 2);
        editor.move_end();
        editor.ensure_visible(5, 2);

        assert_eq!(editor.scroll_row(), 5);
    }
}
