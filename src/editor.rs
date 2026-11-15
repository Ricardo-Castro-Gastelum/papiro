//! Estado del texto, edición, cursor y desplazamiento del área visible.

use std::path::{Path, PathBuf};

use anyhow::Result;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::file;

/// Ancho de un tabulador al mostrarlo. La tecla Tab inserta dos espacios, pero
/// los tabuladores que ya existan en un archivo se alinean a paradas de cuatro
/// columnas.
pub const TAB_DISPLAY_WIDTH: usize = 4;

/// Cada línea se guarda como una cadena UTF-8. El cursor se expresa en
/// índices de grafemas, no de bytes, para que los acentos y los emojis
/// compuestos se traten como una sola unidad.
type Line = String;

pub struct Editor {
    lines: Vec<Line>,
    path: Option<PathBuf>,
    cursor_line: usize,
    cursor_column: usize,
    scroll_line: usize,
    scroll_column: usize,
    viewport_height: usize,
    /// Cambios pendientes desde la última carga o guardado.
    modified: bool,
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
        let lines = text.split('\n').map(str::to_owned).collect();

        Self {
            lines,
            path,
            cursor_line: 0,
            cursor_column: 0,
            scroll_line: 0,
            scroll_column: 0,
            viewport_height: 0,
            modified: false,
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

    /// Indica si hay cambios pendientes de guardar.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Marca el contenido actual como guardado.
    pub fn mark_saved(&mut self) {
        self.modified = false;
    }

    pub fn scroll_line(&self) -> usize {
        self.scroll_line
    }

    pub fn scroll_column(&self) -> usize {
        self.scroll_column
    }

    /// Columna visual del cursor, contando tabuladores y caracteres de doble
    /// ancho. Es distinta de `cursor_column`, que cuenta grafemas.
    pub fn cursor_cell(&self) -> usize {
        cell_width(&self.lines[self.cursor_line], self.cursor_column)
    }

    pub fn move_up(&mut self) {
        self.cursor_line = self.cursor_line.saturating_sub(1);
        self.clamp_column();
    }

    pub fn move_down(&mut self) {
        if self.cursor_line + 1 < self.lines.len() {
            self.cursor_line += 1;
        }
        self.clamp_column();
    }

    pub fn move_left(&mut self) {
        self.cursor_column = self.cursor_column.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        if self.cursor_column < grapheme_count(&self.lines[self.cursor_line]) {
            self.cursor_column += 1;
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_column = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_column = grapheme_count(&self.lines[self.cursor_line]);
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
    }

    pub fn page_up(&mut self) {
        let step = self.page_step();
        self.cursor_line = self.cursor_line.saturating_sub(step);
        self.clamp_column();
    }

    pub fn page_down(&mut self) {
        let step = self.page_step();
        self.cursor_line = (self.cursor_line + step).min(self.lines.len() - 1);
        self.clamp_column();
    }

    fn page_step(&self) -> usize {
        self.viewport_height.saturating_sub(1).max(1)
    }

    fn clamp_column(&mut self) {
        self.cursor_column = self
            .cursor_column
            .min(grapheme_count(&self.lines[self.cursor_line]));
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
            let line = &mut self.lines[self.cursor_line];
            let start = grapheme_offset(line, self.cursor_column);
            let end = grapheme_offset(line, self.cursor_column + 1);
            line.replace_range(start..end, "");
            self.modified = true;
        } else if self.cursor_line + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_line + 1);
            self.lines[self.cursor_line].push_str(&next);
            self.modified = true;
        }
    }

    /// Ajusta el desplazamiento para que el cursor quede dentro del área de
    /// texto. Se llama en cada fotograma, después de conocer el tamaño real.
    pub fn ensure_visible(&mut self, width: usize, height: usize) {
        self.viewport_height = height;

        if height == 0 {
            return;
        }

        if self.cursor_line < self.scroll_line {
            self.scroll_line = self.cursor_line;
        } else if self.cursor_line >= self.scroll_line + height {
            self.scroll_line = self.cursor_line + 1 - height;
        }

        if width == 0 {
            return;
        }

        let cursor_cell = self.cursor_cell();
        if cursor_cell < self.scroll_column {
            self.scroll_column = cursor_cell;
        } else if cursor_cell >= self.scroll_column + width {
            self.scroll_column = cursor_cell + 1 - width;
        }
    }
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

/// Convierte una línea interna en el texto que se dibuja. Los tabuladores se
/// expanden con paradas fijas y los caracteres de control no llegan nunca a la
/// terminal: se muestran como `�` para evitar secuencias de escape ajenas.
pub fn display_line(line: &str) -> String {
    let mut rendered = String::new();
    let mut column = 0;

    for grapheme in line.graphemes(true) {
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
    use super::{Editor, cell_width, display_line, grapheme_count};

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
        assert_eq!(display_line("a\tb"), "a   b");
        assert_eq!(cell_width("a\tb", 3), 5);
    }

    #[test]
    fn calcula_la_anchura_de_emojis() {
        assert_eq!(cell_width("👋🏽a", 2), 3);
    }

    #[test]
    fn sustituye_caracteres_de_control() {
        assert_eq!(display_line("\u{1b}"), "�");
        assert_eq!(cell_width("\u{1b}", 1), 1);
    }

    #[test]
    fn mantiene_el_cursor_visible_al_desplazarse() {
        let mut editor = Editor::from_text("uno\ndos\ntres\ncuatro\ncinco", None);

        for _ in 0..4 {
            editor.move_down();
        }
        editor.ensure_visible(10, 2);

        assert_eq!(editor.scroll_line(), 3);
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
}
