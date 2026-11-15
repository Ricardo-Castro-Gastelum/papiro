//! Estado de la aplicación, edición y bucle de eventos.

use std::io::Stdout;
use std::path::Path;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;

use crate::editor::Editor;
use crate::{file, markdown, render};

const QUIT_WARNING: &str = "Cambios sin guardar · Ctrl+S guardar · Ctrl+Q salir sin guardar";

pub struct App {
    pub editor: Editor,
    running: bool,
    prompt: Option<Prompt>,
    message: Option<String>,
    last_save_name: String,
    view: Option<View>,
    /// Búsqueda incremental activa mientras el indicador inferior está
    /// abierto.
    search: Option<Search>,
    /// Último texto buscado, para repetir la búsqueda con `F3`.
    last_search: String,
    /// Esperando confirmación para salir con cambios sin guardar.
    confirm_quit: bool,
}

/// Contenido ya renderizado del modo Vista. Se guarda para no volver a
/// analizar el documento en cada fotograma.
#[derive(Default)]
struct View {
    lines: Vec<Line<'static>>,
    width: u16,
    rendered: bool,
    scroll: usize,
    viewport_height: usize,
}

/// Motivo de la línea de entrada inferior.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    SaveAs,
    Search,
}

/// Texto que se está escribiendo en la línea inferior.
struct Prompt {
    kind: PromptKind,
    input: String,
}

/// Coincidencia dentro de una línea lógica, en índices de grafemas.
#[derive(Clone, Copy)]
struct SearchMatch {
    line: usize,
    start: usize,
    end: usize,
}

/// Estado de la búsqueda incremental. El origen fijo permite que, al seguir
/// escribiendo, la primera coincidencia se busque siempre desde el mismo
/// punto y no desde donde saltó la anterior.
struct Search {
    query: String,
    matches: Vec<SearchMatch>,
    current: Option<usize>,
    origin_line: usize,
    origin_column: usize,
}

impl App {
    pub fn new(editor: Editor) -> Self {
        Self {
            editor,
            running: true,
            prompt: None,
            message: None,
            last_save_name: String::new(),
            view: None,
            search: None,
            last_search: String::new(),
            confirm_quit: false,
        }
    }

    /// Dibuja y espera el siguiente evento. Solo se redibuja cuando llega una
    /// tecla o cambia el tamaño de la terminal.
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        while self.running {
            terminal.draw(|frame| render::draw(frame, self))?;

            match event::read().context("No se pudo leer un evento de la terminal")? {
                Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        Ok(())
    }

    pub fn prompt_input(&self) -> Option<&str> {
        self.prompt.as_ref().map(|prompt| prompt.input.as_str())
    }

    pub fn prompt_kind(&self) -> Option<PromptKind> {
        self.prompt.as_ref().map(|prompt| prompt.kind)
    }

    /// Texto breve con la posición de la coincidencia actual, para mostrarlo
    /// junto al indicador de búsqueda.
    pub fn search_status(&self) -> Option<String> {
        let search = self.search.as_ref()?;

        if search.query.is_empty() {
            return None;
        }

        if search.matches.is_empty() {
            return Some("sin coincidencias".to_owned());
        }

        let current = search.current.map_or(0, |index| index + 1);
        Some(format!("{current}/{}", search.matches.len()))
    }

    /// Coincidencia que debe resaltarse en el modo Fuente.
    pub fn current_match(&self) -> Option<(usize, usize, usize)> {
        let search = self.search.as_ref()?;
        let found = search.matches.get(search.current?)?;

        Some((found.line, found.start, found.end))
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn is_view_mode(&self) -> bool {
        self.view.is_some()
    }

    /// Prepara el modo Vista para el ancho actual. Solo vuelve a analizar el
    /// documento cuando cambia el ancho de la terminal.
    pub fn ensure_view(&mut self, width: u16, height: usize) {
        let Some(view) = &mut self.view else {
            return;
        };

        if !view.rendered || view.width != width {
            let content = self.editor.content();
            view.lines = markdown::render(&content, width);
            view.width = width;
            view.rendered = true;
        }

        view.viewport_height = height;
        clamp_view(view);
    }

    pub fn view_lines(&self) -> &[Line<'static>] {
        self.view.as_ref().map_or(&[], |view| view.lines.as_slice())
    }

    pub fn view_scroll(&self) -> usize {
        self.view.as_ref().map_or(0, |view| view.scroll)
    }

    fn toggle_view(&mut self) {
        self.editor.break_undo_group();
        self.view = match self.view.take() {
            Some(_) => None,
            None => Some(View::default()),
        };
    }

    fn scroll_view_up(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = view.scroll.saturating_sub(1);
        }
    }

    fn scroll_view_down(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = (view.scroll + 1).min(max_scroll(view));
        }
    }

    fn page_view_up(&mut self) {
        if let Some(view) = &mut self.view {
            let step = view.viewport_height.saturating_sub(1).max(1);
            view.scroll = view.scroll.saturating_sub(step);
        }
    }

    fn page_view_down(&mut self) {
        if let Some(view) = &mut self.view {
            let step = view.viewport_height.saturating_sub(1).max(1);
            view.scroll = (view.scroll + step).min(max_scroll(view));
        }
    }

    fn view_home(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = 0;
        }
    }

    fn view_end(&mut self) {
        if let Some(view) = &mut self.view {
            view.scroll = max_scroll(view);
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if self.confirm_quit {
            self.handle_quit_confirmation(key);
            return;
        }

        if is_quit(&key) {
            self.request_quit();
            return;
        }

        self.message = None;

        if self.prompt.is_some() {
            self.handle_prompt_key(key);
            return;
        }

        if is_save(&key) {
            self.begin_save();
            return;
        }

        if is_search(&key) {
            self.begin_search();
            return;
        }

        if is_search_next(&key) {
            let forward = !key.modifiers.contains(KeyModifiers::SHIFT);
            self.repeat_search(forward);
            return;
        }

        if is_view_toggle(&key) {
            self.toggle_view();
            return;
        }

        if self.view.is_some() {
            match key.code {
                KeyCode::Up => self.scroll_view_up(),
                KeyCode::Down => self.scroll_view_down(),
                KeyCode::PageUp => self.page_view_up(),
                KeyCode::PageDown => self.page_view_down(),
                KeyCode::Home => self.view_home(),
                KeyCode::End => self.view_end(),
                KeyCode::Esc => self.toggle_view(),
                _ => {}
            }
            return;
        }

        if is_undo(&key) {
            self.editor.undo();
            return;
        }

        if is_redo(&key) {
            self.editor.redo();
            return;
        }

        let control = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Enter => self.editor.insert_newline(),
            KeyCode::Tab => self.editor.insert_tab(),
            KeyCode::Backspace => self.editor.backspace(),
            KeyCode::Delete => self.editor.delete(),
            KeyCode::Up => self.editor.move_up(),
            KeyCode::Down => self.editor.move_down(),
            KeyCode::Left if control => self.editor.move_word_left(),
            KeyCode::Right if control => self.editor.move_word_right(),
            KeyCode::Left => self.editor.move_left(),
            KeyCode::Right => self.editor.move_right(),
            KeyCode::Home => self.editor.move_home(),
            KeyCode::End => self.editor.move_end(),
            KeyCode::PageUp => self.editor.page_up(),
            KeyCode::PageDown => self.editor.page_down(),
            KeyCode::Char(character) if is_text_input(&key) && !character.is_control() => {
                self.editor.insert_char(character);
            }
            _ => {}
        }
    }

    /// Sale de inmediato si no hay cambios pendientes; en caso contrario pide
    /// confirmación en la línea inferior.
    fn request_quit(&mut self) {
        if self.editor.is_modified() {
            self.prompt = None;
            self.confirm_quit = true;
            self.message = Some(QUIT_WARNING.to_owned());
        } else {
            self.running = false;
        }
    }

    /// Mientras se espera confirmación, `Ctrl+Q` descarta los cambios,
    /// `Ctrl+S` guarda y cualquier otra tecla (incluido `Esc`) cancela.
    fn handle_quit_confirmation(&mut self, key: KeyEvent) {
        if is_quit(&key) {
            self.running = false;
        } else if is_save(&key) {
            self.confirm_quit = false;
            self.message = None;
            self.begin_save();
        } else {
            self.confirm_quit = false;
            self.message = None;
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        match self.prompt_kind() {
            Some(PromptKind::SaveAs) => self.handle_save_prompt_key(key),
            Some(PromptKind::Search) => self.handle_search_prompt_key(key),
            None => {}
        }
    }

    fn handle_save_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.confirm_save_as(),
            KeyCode::Esc => self.prompt = None,
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    remove_last_grapheme(&mut prompt.input);
                }
            }
            KeyCode::Char(character) if is_text_input(&key) && !character.is_control() => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(character);
                }
            }
            _ => {}
        }
    }

    fn handle_search_prompt_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.step_search(false)
            }
            KeyCode::Enter => self.step_search(true),
            KeyCode::Esc => self.close_search(),
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    remove_last_grapheme(&mut prompt.input);
                }
                self.refresh_search();
            }
            KeyCode::Char(character) if is_text_input(&key) && !character.is_control() => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.input.push(character);
                }
                self.refresh_search();
            }
            _ => {}
        }
    }

    /// Abre la búsqueda desde el cursor. Si se estaba en Vista, vuelve a
    /// Fuente para poder saltar entre coincidencias.
    fn begin_search(&mut self) {
        if self.view.is_some() {
            self.toggle_view();
        }

        let origin_line = self.editor.cursor_line();
        let origin_column = self.editor.cursor_column();

        self.search = Some(Search {
            query: String::new(),
            matches: Vec::new(),
            current: None,
            origin_line,
            origin_column,
        });
        self.prompt = Some(Prompt {
            kind: PromptKind::Search,
            input: String::new(),
        });
        self.message = None;
    }

    /// Recalcula las coincidencias al cambiar el texto buscado y salta a la
    /// primera que aparece desde el origen de la búsqueda.
    fn refresh_search(&mut self) {
        let Some(input) = self.prompt.as_ref().map(|prompt| prompt.input.clone()) else {
            return;
        };
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let origin_line = search.origin_line;
        let origin_column = search.origin_column;
        let matches = find_matches(self.editor.lines(), &input);

        let current = if input.is_empty() {
            None
        } else {
            matches
                .iter()
                .position(|candidate| {
                    candidate.line > origin_line
                        || (candidate.line == origin_line && candidate.start >= origin_column)
                })
                .or_else(|| (!matches.is_empty()).then_some(0))
        };

        if let Some(search) = self.search.as_mut() {
            search.query = input;
            search.matches = matches;
            search.current = current;
        }

        self.jump_to_current_match();
    }

    fn step_search(&mut self, forward: bool) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        if search.matches.is_empty() {
            return;
        }

        let total = search.matches.len();
        let next = match (search.current, forward) {
            (Some(index), true) => (index + 1) % total,
            (Some(index), false) => (index + total - 1) % total,
            (None, true) => 0,
            (None, false) => total - 1,
        };

        if let Some(search) = self.search.as_mut() {
            search.current = Some(next);
        }
        self.jump_to_current_match();
    }

    fn jump_to_current_match(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let Some(index) = search.current else {
            return;
        };
        let Some(found) = search.matches.get(index) else {
            return;
        };
        let (line, start) = (found.line, found.start);

        self.editor.set_cursor(line, start);
    }

    fn close_search(&mut self) {
        if let Some(search) = self.search.take()
            && !search.query.is_empty()
        {
            self.last_search = search.query;
        }
        self.prompt = None;
    }

    /// Repite la última búsqueda desde el cursor con `F3` o `Shift+F3`.
    fn repeat_search(&mut self, forward: bool) {
        if self.last_search.is_empty() || self.prompt.is_some() {
            return;
        }

        if self.view.is_some() {
            self.toggle_view();
        }

        let line = self.editor.cursor_line();
        let column = self.editor.cursor_column();
        let query = self.last_search.clone();
        let matches = find_matches(self.editor.lines(), &query);

        if matches.is_empty() {
            self.message = Some(format!("Sin coincidencias para «{query}»."));
            return;
        }

        let target = if forward {
            matches
                .iter()
                .position(|candidate| {
                    candidate.line > line || (candidate.line == line && candidate.start > column)
                })
                .unwrap_or(0)
        } else {
            matches
                .iter()
                .rposition(|candidate| {
                    candidate.line < line || (candidate.line == line && candidate.start < column)
                })
                .unwrap_or(matches.len() - 1)
        };

        let found = matches[target];
        self.editor.set_cursor(found.line, found.start);
        self.message = Some(format!("Coincidencia {} de {}", target + 1, matches.len()));
    }

    fn begin_save(&mut self) {
        if let Some(path) = self.editor.path().map(Path::to_path_buf) {
            self.save_to(&path);
        } else {
            self.prompt = Some(Prompt {
                kind: PromptKind::SaveAs,
                input: std::mem::take(&mut self.last_save_name),
            });
        }
    }

    fn confirm_save_as(&mut self) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };

        let name = prompt.input.trim();
        if name.is_empty() {
            self.message = Some("El nombre del archivo no puede estar vacío.".to_owned());
            return;
        }

        self.last_save_name = name.to_owned();
        self.save_to(Path::new(name));
    }

    fn save_to(&mut self, path: &Path) {
        match file::save_file(path, &self.editor.content()) {
            Ok(()) => {
                self.editor.set_path(path.to_path_buf());
                self.editor.mark_saved();
                self.message = Some("Guardado.".to_owned());
            }
            Err(error) => self.message = Some(format!("{error:#}")),
        }
    }
}

fn is_quit(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    (control && matches!(key.code, KeyCode::Char('q' | 'Q')))
        || matches!(key.code, KeyCode::Char('\u{11}'))
}

fn is_save(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    (control && matches!(key.code, KeyCode::Char('s' | 'S')))
        || matches!(key.code, KeyCode::Char('\u{13}'))
}

fn is_view_toggle(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    (control && matches!(key.code, KeyCode::Char('v' | 'V')))
        || matches!(key.code, KeyCode::Char('\u{16}'))
}

fn is_search(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    (control && matches!(key.code, KeyCode::Char('f' | 'F')))
        || matches!(key.code, KeyCode::Char('\u{06}'))
}

fn is_search_next(key: &KeyEvent) -> bool {
    matches!(key.code, KeyCode::F(3))
}

/// Busca `query` en cada línea sin distinguir mayúsculas. La comparación se
/// hace por grafemas para que los acentos y emojis no desalineen los índices.
fn find_matches(lines: &[String], query: &str) -> Vec<SearchMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let needle = lowercase_graphemes(query);
    if needle.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();

    for (line_index, line) in lines.iter().enumerate() {
        let haystack = lowercase_graphemes(line);
        if haystack.len() < needle.len() {
            continue;
        }

        let mut start = 0;
        while start + needle.len() <= haystack.len() {
            if haystack[start..].starts_with(&needle) {
                matches.push(SearchMatch {
                    line: line_index,
                    start,
                    end: start + needle.len(),
                });
                start += needle.len();
            } else {
                start += 1;
            }
        }
    }

    matches
}

fn lowercase_graphemes(text: &str) -> Vec<String> {
    text.graphemes(true).map(str::to_lowercase).collect()
}

fn is_undo(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    control && !shift && matches!(key.code, KeyCode::Char('z' | 'Z' | '\u{1a}'))
}

fn is_redo(key: &KeyEvent) -> bool {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    (control && matches!(key.code, KeyCode::Char('y' | 'Y' | '\u{19}')))
        || (control && shift && matches!(key.code, KeyCode::Char('z' | 'Z')))
}

fn max_scroll(view: &View) -> usize {
    view.lines.len().saturating_sub(view.viewport_height)
}

fn clamp_view(view: &mut View) {
    view.scroll = view.scroll.min(max_scroll(view));
}

/// En algunas distribuciones, AltGr se representa como Ctrl+Alt. Esos
/// caracteres deben poder escribirse; Ctrl sin Alt queda reservado para los
/// atajos.
fn is_text_input(key: &KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT)
}

fn remove_last_grapheme(text: &mut String) {
    if let Some((index, _)) = text.grapheme_indices(true).next_back() {
        text.truncate(index);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{App, PromptKind, find_matches};
    use crate::editor::Editor;

    #[test]
    fn pide_un_nombre_y_guarda_un_archivo_nuevo() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));

        app.begin_save();
        let prompt = app.prompt.as_mut().unwrap();
        prompt.input = path.to_string_lossy().into_owned();
        app.confirm_save_as();

        assert_eq!(app.message(), Some("Guardado."));
        assert!(app.prompt_input().is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola");
        assert_eq!(app.editor.path(), Some(path.as_path()));
    }

    #[test]
    fn rechaza_un_nombre_vacio() {
        let mut app = App::new(Editor::from_text("Hola", None));

        app.begin_save();
        app.prompt.as_mut().unwrap().input.clear();
        app.confirm_save_as();

        assert!(app.prompt_input().is_none());
        assert!(app.message().is_some());
    }

    #[test]
    fn alterna_entre_fuente_y_vista() {
        let mut app = App::new(Editor::from_text("# Título", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(app.is_view_mode());
        assert_eq!(app.editor.content(), "# Título");

        app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        assert!(!app.is_view_mode());
    }

    #[test]
    fn escape_vuelve_de_la_vista_a_la_fuente() {
        let mut app = App::new(Editor::from_text("# Título", None));

        app.toggle_view();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(!app.is_view_mode());
    }

    #[test]
    fn guardar_limpia_la_marca_de_cambios() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));

        app.editor.move_end();
        app.editor.insert_char('!');
        assert!(app.editor.is_modified());

        app.save_to(&path);

        assert_eq!(app.message(), Some("Guardado."));
        assert!(!app.editor.is_modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola!");
    }

    #[test]
    fn salir_sin_cambios_no_pide_confirmacion() {
        let mut app = App::new(Editor::from_text("Hola", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));

        assert!(!app.running);
    }

    #[test]
    fn pide_confirmacion_antes_de_salir_con_cambios() {
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(app.running);
        assert!(app.confirm_quit);
        assert!(app.message().is_some());

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(!app.running);
    }

    #[test]
    fn escape_cancela_la_confirmacion_de_salida() {
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        assert!(app.running);
        assert!(!app.confirm_quit);
        assert!(app.message().is_none());
        assert_eq!(app.editor.content(), "Hola!");
    }

    #[test]
    fn guardar_desde_la_confirmacion_no_sale() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nota.md");
        let mut app = App::new(Editor::from_text("Hola", None));
        app.editor.move_end();
        app.editor.insert_char('!');
        app.editor.set_path(path.clone());

        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(app.running);
        assert!(!app.confirm_quit);
        assert!(!app.editor.is_modified());
        assert_eq!(fs::read_to_string(&path).unwrap(), "Hola!");
    }

    #[test]
    fn desplaza_la_vista_y_acota_el_final() {
        let text = (0..40)
            .map(|index| format!("línea {index}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut app = App::new(Editor::from_text(&text, None));

        app.toggle_view();
        app.ensure_view(30, 5);

        app.scroll_view_down();
        assert_eq!(app.view_scroll(), 1);

        app.view_end();
        let maximum = app.view_lines().len().saturating_sub(5);
        assert_eq!(app.view_scroll(), maximum);

        app.view_home();
        assert_eq!(app.view_scroll(), 0);
    }

    #[test]
    fn deshace_y_rehace_con_los_atajos() {
        let mut app = App::new(Editor::from_text("", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert_eq!(app.editor.content(), "ab");

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(app.editor.content(), "");

        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(app.editor.content(), "ab");
    }

    #[test]
    fn control_mas_shift_z_tambien_rehace() {
        let mut app = App::new(Editor::from_text("", None));

        app.editor.insert_char('a');
        app.editor.undo();
        assert_eq!(app.editor.content(), "");

        app.handle_key(KeyEvent::new(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ));
        assert_eq!(app.editor.content(), "a");
    }

    #[test]
    fn deshacer_no_cambia_el_modo_vista() {
        let mut app = App::new(Editor::from_text("hola", None));
        app.editor.insert_char('!');
        app.toggle_view();

        app.handle_key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));

        assert!(app.is_view_mode());
        assert_eq!(app.editor.content(), "!hola");
    }

    #[test]
    fn busca_y_salta_entre_coincidencias() {
        let mut app = App::new(Editor::from_text("uno dos uno dos", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(matches!(app.prompt_kind(), Some(PromptKind::Search)));

        for character in "dos".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(app.editor.cursor_column(), 4);
        assert_eq!(app.search_status().as_deref(), Some("1/2"));
        assert_eq!(app.current_match(), Some((0, 4, 7)));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.search_status().as_deref(), Some("2/2"));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.search_status().as_deref(), Some("2/2"));

        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.prompt_kind().is_none());
        assert_eq!(app.current_match(), None);
        assert_eq!(app.editor.cursor_column(), 4);
    }

    #[test]
    fn f3_repite_la_ultima_busqueda() {
        let mut app = App::new(Editor::from_text("uno dos uno dos", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for character in "dos".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 4);

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE));
        assert_eq!(app.editor.cursor_column(), 12);
        assert_eq!(app.message(), Some("Coincidencia 2 de 2"));

        app.handle_key(KeyEvent::new(KeyCode::F(3), KeyModifiers::SHIFT));
        assert_eq!(app.editor.cursor_column(), 4);
    }

    #[test]
    fn buscar_desde_vista_vuelve_a_fuente() {
        let mut app = App::new(Editor::from_text("# Título", None));
        app.toggle_view();

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));

        assert!(!app.is_view_mode());
        assert!(matches!(app.prompt_kind(), Some(PromptKind::Search)));
    }

    #[test]
    fn avisa_cuando_la_busqueda_no_encuentra_nada() {
        let mut app = App::new(Editor::from_text("hola", None));

        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        for character in "zzz".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }

        assert_eq!(app.search_status().as_deref(), Some("sin coincidencias"));
        assert_eq!(app.current_match(), None);
    }

    #[test]
    fn encuentra_coincidencias_sin_distinguir_mayusculas() {
        let lines = vec!["Hola Mundo".to_owned(), "mundo mundo".to_owned()];

        let matches = find_matches(&lines, "mundo");

        assert_eq!(matches.len(), 3);
        assert_eq!(
            (matches[0].line, matches[0].start, matches[0].end),
            (0, 5, 10)
        );
        assert_eq!(
            (matches[1].line, matches[1].start, matches[1].end),
            (1, 0, 5)
        );
        assert_eq!(
            (matches[2].line, matches[2].start, matches[2].end),
            (1, 6, 11)
        );
    }

    #[test]
    fn busca_acentos_por_grafemas() {
        let lines = vec!["El año pasado".to_owned()];

        let matches = find_matches(&lines, "AÑO");

        assert_eq!(matches.len(), 1);
        assert_eq!((matches[0].start, matches[0].end), (3, 6));
    }
}
