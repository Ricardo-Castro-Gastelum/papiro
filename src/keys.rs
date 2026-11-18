use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    #[default]
    Default,
    Vim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    Save,
    Quit,
    View,
    Search,
    SearchNext,
    SearchPrev,
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum KeyName {
    Char(char),
    Function(u8),
    Esc,
    Enter,
    Tab,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpec {
    ctrl: bool,
    shift: bool,
    alt: bool,
    name: KeyName,
}

impl KeySpec {
    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split('+').collect::<Vec<_>>();
        let key = parts.pop()?;
        if key.is_empty() {
            return None;
        }

        let mut ctrl = false;
        let mut shift = false;
        let mut alt = false;
        for modifier in parts {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "shift" => shift = true,
                "alt" => alt = true,
                _ => return None,
            }
        }

        let mut name = named_key(key)?;
        if (ctrl || shift || alt)
            && let KeyName::Char(character) = &mut name
        {
            *character = character.to_ascii_lowercase();
        }

        Some(Self {
            ctrl,
            shift,
            alt,
            name,
        })
    }

    fn from_char(character: char) -> Option<Self> {
        Some(Self {
            ctrl: false,
            shift: false,
            alt: false,
            name: KeyName::Char(character),
        })
    }

    fn from_event(key: &KeyEvent) -> Option<Self> {
        let mut ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        let name = match key.code {
            KeyCode::Char(character) => {
                let character = if character.is_ascii_control() {
                    ctrl = true;
                    control_letter(character)?
                } else if ctrl || alt {
                    character.to_ascii_lowercase()
                } else {
                    character
                };
                KeyName::Char(character)
            }
            KeyCode::F(number) => KeyName::Function(number),
            KeyCode::Esc => KeyName::Esc,
            KeyCode::Enter => KeyName::Enter,
            KeyCode::Tab => KeyName::Tab,
            KeyCode::Backspace => KeyName::Backspace,
            KeyCode::Delete => KeyName::Delete,
            KeyCode::Home => KeyName::Home,
            KeyCode::End => KeyName::End,
            KeyCode::PageUp => KeyName::PageUp,
            KeyCode::PageDown => KeyName::PageDown,
            KeyCode::Up => KeyName::Up,
            KeyCode::Down => KeyName::Down,
            KeyCode::Left => KeyName::Left,
            KeyCode::Right => KeyName::Right,
            _ => return None,
        };

        Some(Self {
            ctrl,
            shift,
            alt,
            name,
        })
    }

    fn insert_safe(&self) -> bool {
        if self.ctrl || self.alt {
            return true;
        }
        !matches!(self.name, KeyName::Char(_))
    }

    fn matches(&self, key: &KeyEvent) -> bool {
        let Some(other) = Self::from_event(key) else {
            return false;
        };

        if self.ctrl != other.ctrl || self.alt != other.alt || self.name != other.name {
            return false;
        }

        let letter = matches!(self.name, KeyName::Char(c) if c.is_ascii_alphabetic());
        if letter {
            !self.shift || other.shift
        } else {
            self.shift == other.shift
        }
    }
}

fn control_letter(character: char) -> Option<char> {
    match character {
        '\u{01}' => Some('a'),
        '\u{04}' => Some('d'),
        '\u{05}' => Some('e'),
        '\u{06}' => Some('f'),
        '\u{0b}' => Some('k'),
        '\u{0e}' => Some('n'),
        '\u{10}' => Some('p'),
        '\u{11}' => Some('q'),
        '\u{12}' => Some('r'),
        '\u{13}' => Some('s'),
        '\u{16}' => Some('v'),
        '\u{19}' => Some('y'),
        '\u{1a}' => Some('z'),
        _ => None,
    }
}

fn is_function(key: &str) -> bool {
    key.len() > 1
        && key.starts_with(['f', 'F'])
        && key[1..].chars().all(|character| character.is_ascii_digit())
}

fn named_key(key: &str) -> Option<KeyName> {
    if is_function(key) {
        let number = key[1..].parse::<u8>().ok()?;
        return (1..=12)
            .contains(&number)
            .then_some(KeyName::Function(number));
    }

    if key.chars().count() == 1 {
        return Some(KeyName::Char(key.chars().next()?));
    }

    match key.to_ascii_lowercase().as_str() {
        "esc" | "escape" => Some(KeyName::Esc),
        "enter" | "return" => Some(KeyName::Enter),
        "tab" => Some(KeyName::Tab),
        "backspace" => Some(KeyName::Backspace),
        "delete" | "del" => Some(KeyName::Delete),
        "home" => Some(KeyName::Home),
        "end" => Some(KeyName::End),
        "pageup" => Some(KeyName::PageUp),
        "pagedown" => Some(KeyName::PageDown),
        "up" => Some(KeyName::Up),
        "down" => Some(KeyName::Down),
        "left" => Some(KeyName::Left),
        "right" => Some(KeyName::Right),
        _ => None,
    }
}

fn parse_binding(value: &str) -> Option<Vec<KeySpec>> {
    let mut specs = Vec::new();

    for token in value.split_whitespace() {
        if token.contains('+') || named_key(token).is_some() {
            specs.push(KeySpec::parse(token)?);
        } else {
            for character in token.chars() {
                specs.push(KeySpec::from_char(character)?);
            }
        }
    }

    (!specs.is_empty()).then_some(specs)
}

#[derive(Clone)]
struct Binding {
    sequence: Vec<KeySpec>,
    command: Command,
    normal_only: bool,
}

#[derive(Clone)]
pub struct Bindings {
    entries: Vec<Binding>,
}

impl Bindings {
    fn preset(preset: Preset) -> Self {
        let mut entries = Vec::new();

        for (command, values) in defaults(preset) {
            for value in values {
                if let Some(sequence) = parse_binding(value) {
                    let normal_only = sequence.iter().any(|spec| !spec.insert_safe());
                    entries.push(Binding {
                        sequence,
                        command,
                        normal_only,
                    });
                }
            }
        }

        Self { entries }
    }

    fn set(&mut self, command: Command, sequences: Vec<Vec<KeySpec>>) {
        self.entries.retain(|binding| binding.command != command);

        for sequence in sequences {
            let normal_only = sequence.iter().any(|spec| !spec.insert_safe());
            self.entries.push(Binding {
                sequence,
                command,
                normal_only,
            });
        }
    }

    pub fn matches(&self, key: &KeyEvent, command: Command, insert_mode: bool) -> bool {
        self.entries.iter().any(|binding| {
            binding.command == command
                && (!binding.normal_only || !insert_mode)
                && binding.sequence.len() == 1
                && binding.sequence[0].matches(key)
        })
    }

    pub fn resolve(
        &self,
        key: &KeyEvent,
        insert_mode: bool,
        pending: &mut Vec<KeySpec>,
    ) -> Option<Command> {
        let spec = KeySpec::from_event(key)?;
        let was_pending = !pending.is_empty();
        let mut candidate = std::mem::take(pending);
        candidate.push(spec.clone());

        let visible = |binding: &Binding| !binding.normal_only || !insert_mode;

        if let Some(binding) = self
            .entries
            .iter()
            .find(|binding| visible(binding) && binding.sequence == candidate)
        {
            return Some(binding.command);
        }

        if self.entries.iter().any(|binding| {
            visible(binding)
                && binding.sequence.len() > candidate.len()
                && binding.sequence.starts_with(&candidate)
        }) {
            *pending = candidate;
            return None;
        }

        if was_pending {
            let single = vec![spec];
            if let Some(binding) = self.entries.iter().find(|binding| {
                visible(binding) && binding.sequence.len() == 1 && binding.sequence[0].matches(key)
            }) {
                return Some(binding.command);
            }

            if self.entries.iter().any(|binding| {
                visible(binding) && binding.sequence.len() > 1 && binding.sequence[0].matches(key)
            }) {
                *pending = single;
            }
        }

        None
    }
}

fn defaults(preset: Preset) -> Vec<(Command, &'static [&'static str])> {
    match preset {
        Preset::Default => vec![
            (Command::Save, &["Ctrl+S"]),
            (Command::Quit, &["Ctrl+Q"]),
            (Command::View, &["Ctrl+V"]),
            (Command::Search, &["Ctrl+F"]),
            (Command::SearchNext, &["F3"]),
            (Command::SearchPrev, &["Shift+F3"]),
            (Command::Undo, &["Ctrl+Z"]),
            (Command::Redo, &["Ctrl+Y", "Ctrl+Shift+Z"]),
        ],
        Preset::Vim => vec![
            (Command::Save, &["Ctrl+S", ":w"]),
            (Command::Quit, &["Ctrl+Q", ":q"]),
            (Command::View, &["Ctrl+V"]),
            (Command::Search, &["Ctrl+F", "/"]),
            (Command::SearchNext, &["F3", "n"]),
            (Command::SearchPrev, &["Shift+F3", "N"]),
            (Command::Undo, &["Ctrl+Z", "u"]),
            (Command::Redo, &["Ctrl+Y", "Ctrl+Shift+Z", "Ctrl+R"]),
        ],
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ConfigFile {
    keymap: Option<String>,
    bindings: HashMap<String, Vec<String>>,
}

#[derive(Clone)]
pub struct Settings {
    preset: Preset,
    bindings: Bindings,
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_toml("", None, "").0
    }
}

impl Settings {
    pub fn load(cli: Option<Preset>) -> (Self, Vec<String>) {
        let Some(path) = config_path() else {
            return (Self::new(cli.unwrap_or_default(), cli), Vec::new());
        };

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return (Self::new(cli.unwrap_or_default(), cli), Vec::new());
            }
            Err(error) => {
                let message = format!("No se pudo leer {}: {error}", path.display());
                return (Self::new(cli.unwrap_or_default(), cli), vec![message]);
            }
        };

        Self::from_toml(&text, cli, &path.to_string_lossy())
    }

    #[cfg(test)]
    pub fn for_test(text: &str, cli: Option<Preset>) -> Self {
        Self::from_toml(text, cli, "config.toml").0
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn bindings(&self) -> &Bindings {
        &self.bindings
    }

    fn new(preset: Preset, cli: Option<Preset>) -> Self {
        let _ = cli;
        Self {
            preset,
            bindings: Bindings::preset(preset),
        }
    }

    fn from_toml(text: &str, cli: Option<Preset>, source: &str) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let config: ConfigFile = match toml::from_str(text) {
            Ok(config) => config,
            Err(error) => {
                warnings.push(format!("Configuración inválida en {source}: {error}"));
                let preset = cli.unwrap_or_default();
                return (Self::new(preset, cli), warnings);
            }
        };

        let mut preset = cli.unwrap_or_default();
        if let Some(name) = config.keymap.as_deref() {
            match preset_from_name(name) {
                Some(parsed) if cli.is_none() => preset = parsed,
                Some(_) => {}
                None => warnings.push(format!(
                    "Modo de teclado desconocido «{name}»; se usa el predeterminado."
                )),
            }
        }

        let mut bindings = Bindings::preset(preset);

        for (name, values) in config.bindings {
            let Some(command) = command_from_name(&name) else {
                warnings.push(format!(
                    "Acción desconocida «{name}» en {source}; se ignora."
                ));
                continue;
            };

            let mut sequences = Vec::new();
            for value in values {
                match parse_binding(&value) {
                    Some(sequence) => sequences.push(sequence),
                    None => warnings.push(format!(
                        "Atajo inválido «{value}» para {name} en {source}; se ignora."
                    )),
                }
            }

            if sequences.is_empty() {
                warnings.push(format!(
                    "Sin atajos válidos para {name} en {source}; se conservan los predeterminados."
                ));
            } else {
                bindings.set(command, sequences);
            }
        }

        (Self { preset, bindings }, warnings)
    }
}

fn preset_from_name(name: &str) -> Option<Preset> {
    match name.to_ascii_lowercase().as_str() {
        "default" | "normal" => Some(Preset::Default),
        "vim" => Some(Preset::Vim),
        _ => None,
    }
}

fn command_from_name(name: &str) -> Option<Command> {
    match name.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "save" | "write" => Some(Command::Save),
        "quit" | "exit" => Some(Command::Quit),
        "view" | "toggle_view" | "toggle" => Some(Command::View),
        "search" | "find" => Some(Command::Search),
        "search_next" | "next" => Some(Command::SearchNext),
        "search_prev" | "search_previous" | "previous" => Some(Command::SearchPrev),
        "undo" => Some(Command::Undo),
        "redo" => Some(Command::Redo),
        _ => None,
    }
}

fn config_path() -> Option<PathBuf> {
    if let Some(directory) = env::var_os("XDG_CONFIG_HOME")
        && !directory.is_empty()
    {
        return Some(PathBuf::from(directory).join("papiro").join("config.toml"));
    }

    let home = env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }

    Some(
        PathBuf::from(home)
            .join(".config")
            .join("papiro")
            .join("config.toml"),
    )
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::{Bindings, Command, KeySpec, Preset, Settings, parse_binding};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        }
    }

    #[test]
    fn analiza_atajos_con_modificadores() {
        let spec = KeySpec::parse("Ctrl+Shift+Z").unwrap();
        assert_eq!(spec, KeySpec::parse("control+shift+z").unwrap());
        assert!(spec.matches(&key(
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )));

        let save = KeySpec::parse("Ctrl+S").unwrap();
        assert!(save.matches(&key(KeyCode::Char('S'), KeyModifiers::CONTROL)));
        assert!(save.matches(&key(KeyCode::Char('\u{13}'), KeyModifiers::NONE)));
    }

    #[test]
    fn distingue_letras_mayusculas_sin_modificadores() {
        let lower = KeySpec::parse("n").unwrap();
        let upper = KeySpec::parse("N").unwrap();

        assert!(lower.matches(&key(KeyCode::Char('n'), KeyModifiers::NONE)));
        assert!(!lower.matches(&key(KeyCode::Char('N'), KeyModifiers::NONE)));
        assert!(upper.matches(&key(KeyCode::Char('N'), KeyModifiers::NONE)));
    }

    #[test]
    fn analiza_secuencias_y_teclas_nombradas() {
        assert_eq!(
            parse_binding(":w").unwrap(),
            vec![KeySpec::parse(":").unwrap(), KeySpec::parse("w").unwrap()]
        );
        assert_eq!(
            parse_binding("Shift+F3").unwrap(),
            vec![KeySpec::parse("Shift+F3").unwrap()]
        );
        assert!(parse_binding("Ctrl++").is_none());
    }

    #[test]
    fn resuelve_atajos_segun_el_modo() {
        let bindings = Bindings::preset(Preset::Vim);
        let mut pending = Vec::new();

        let first = key(KeyCode::Char(':'), KeyModifiers::NONE);
        assert_eq!(bindings.resolve(&first, false, &mut pending), None);
        assert_eq!(pending.len(), 1);

        let second = key(KeyCode::Char('w'), KeyModifiers::NONE);
        assert_eq!(
            bindings.resolve(&second, false, &mut pending),
            Some(Command::Save)
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn los_atajos_de_texto_no_actuan_al_escribir() {
        let bindings = Bindings::preset(Preset::Vim);
        let mut pending = Vec::new();
        let slash = key(KeyCode::Char('/'), KeyModifiers::NONE);

        assert_eq!(bindings.resolve(&slash, true, &mut pending), None);
        assert!(pending.is_empty());
        assert_eq!(
            bindings.resolve(&slash, false, &mut pending),
            Some(Command::Search)
        );
    }

    #[test]
    fn carga_el_preset_y_los_atajos_desde_toml() {
        let text = "keymap = \"vim\"\n[bindings]\nsave = [\"Ctrl+S\", \":guardar\"]\n";
        let (settings, warnings) = Settings::from_toml(text, None, "config.toml");

        assert_eq!(settings.preset(), Preset::Vim);
        assert!(warnings.is_empty());
        assert!(settings.bindings().matches(
            &key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            Command::Redo,
            true
        ));
        let mut pending = Vec::new();
        assert_eq!(
            settings.bindings().resolve(
                &key(KeyCode::Char(':'), KeyModifiers::NONE),
                false,
                &mut pending
            ),
            None
        );
        let word = "guardar";
        for (index, character) in word.chars().enumerate() {
            let result = settings.bindings().resolve(
                &key(KeyCode::Char(character), KeyModifiers::NONE),
                false,
                &mut pending,
            );
            if index == word.len() - 1 {
                assert_eq!(result, Some(Command::Save));
            } else {
                assert_eq!(result, None);
            }
        }
        assert!(!settings.bindings().matches(
            &key(KeyCode::Char('w'), KeyModifiers::NONE),
            Command::Save,
            false
        ));
    }

    #[test]
    fn la_linea_de_comandos_gana_a_la_configuracion() {
        let (settings, _) =
            Settings::from_toml("keymap = \"vim\"", Some(Preset::Default), "config.toml");
        assert_eq!(settings.preset(), Preset::Default);
    }

    #[test]
    fn advierte_de_configuracion_invalida_y_usa_valores_por_defecto() {
        let (settings, warnings) = Settings::from_toml("keymap = 12", None, "config.toml");

        assert_eq!(settings.preset(), Preset::Default);
        assert!(!warnings.is_empty());
    }
}
