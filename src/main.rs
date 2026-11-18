mod app;
mod editor;
mod file;
mod keys;
mod markdown;
mod render;
mod syntax;
mod theme;

use std::env;
use std::io::{self, Stdout, stdout};
use std::panic;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use crossterm::cursor::Show;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::App;
use crate::editor::Editor;
use crate::keys::{Preset, Settings};

const HELP: &str = "\
Papiro — editor y lector de Markdown para la terminal.

Uso:
  ppr [ARCHIVO]

Argumentos:
  ARCHIVO    Archivo Markdown que se abre. Si no existe, se crea al guardar.
             Sin argumento se abre un búfer sin título.

Opciones:
  -h, --help       Muestra esta ayuda y sale.
  -V, --version    Muestra la versión y sale.
      --keymap     Elige los atajos: default o vim.
      --vim        Usa los atajos de vim (igual que --keymap vim).
      --default    Usa los atajos normales (igual que --keymap default).

Atajos normales:
  Ctrl+S guardar   Ctrl+V vista   Ctrl+F buscar   Ctrl+Q salir
  Ctrl+Z deshacer  Ctrl+Y rehacer F3 buscar siguiente

Atajos vim:
  h/j/k/l mover   w/b palabras   0/$ inicio/fin   x borrar
  i/a/o insertar  Esc normal      :w guardar       :q salir
  / buscar        n/N coincidencia   u deshacer    Ctrl+R rehacer

Ratón:
  Clic coloca el cursor   Rueda desplaza el texto o la vista

Configuración: ~/.config/papiro/config.toml
  También se respeta $XDG_CONFIG_HOME/papiro/config.toml.

Repositorio: https://github.com/Ricardo-Castro-Gastelum/papiro";

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Edit(Option<PathBuf>, Option<Preset>),
    Help,
    Version,
}

fn main() -> ExitCode {
    install_panic_hook();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Papiro: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let (path, cli_preset) = match parse_args()? {
        Action::Edit(path, preset) => (path, preset),
        Action::Help => {
            println!("{HELP}");
            return Ok(());
        }
        Action::Version => {
            println!("ppr {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    };

    let (settings, warnings) = Settings::load(cli_preset);
    for warning in warnings {
        eprintln!("Papiro: {warning}");
    }

    let editor = Editor::open(path)?;
    let mut app = App::with_settings(editor, settings);

    let mut session = TerminalSession::new()?;
    app.run(session.terminal())?;
    drop(session);

    Ok(())
}

fn parse_args() -> Result<Action> {
    parse_args_from(env::args_os().skip(1))
}

fn parse_args_from<I>(args: I) -> Result<Action>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let args = args.into_iter().collect::<Vec<_>>();
    let mut path = None;
    let mut preset = None;
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];
        match argument.to_str() {
            Some("-h" | "--help") => return Ok(Action::Help),
            Some("-V" | "--version") => return Ok(Action::Version),
            Some("--vim" | "-vim") => preset = Some(Preset::Vim),
            Some("--default" | "-default") => preset = Some(Preset::Default),
            Some("--keymap") => {
                index += 1;
                let value = args.get(index).and_then(|value| value.to_str());
                let Some(value) = value else {
                    bail!("--keymap necesita un valor: default o vim.");
                };
                preset = Some(parse_preset(value)?);
            }
            Some(value) if value.starts_with("--keymap=") => {
                preset = Some(parse_preset(&value["--keymap=".len()..])?);
            }
            Some(flag) if flag.starts_with('-') && flag != "-" => {
                bail!("Opción desconocida «{flag}». Prueba «ppr --help».");
            }
            _ => {
                if path.is_some() {
                    bail!("Uso: ppr [ARCHIVO]\nPrueba «ppr --help» para más información.");
                }
                if argument.is_empty() {
                    bail!("La ruta del archivo no puede estar vacía.");
                }
                path = Some(PathBuf::from(argument));
            }
        }
        index += 1;
    }

    Ok(Action::Edit(path, preset))
}

fn parse_preset(value: &str) -> Result<Preset> {
    match value.to_ascii_lowercase().as_str() {
        "default" | "normal" => Ok(Preset::Default),
        "vim" => Ok(Preset::Vim),
        _ => bail!("Modo de teclado desconocido «{value}». Usa default o vim."),
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        enable_raw_mode().context("No se pudo activar el modo raw de la terminal")?;

        let mut output = stdout();
        if let Err(error) = execute!(output, EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(error).context("No se pudo abrir la pantalla alterna");
        }
        if let Err(error) = execute!(output, EnableMouseCapture) {
            let _ = execute!(output, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(error).context("No se pudo activar el ratón");
        }

        let backend = CrosstermBackend::new(output);
        match Terminal::new(backend) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                restore_terminal();
                Err(error).context("No se pudo inicializar la interfaz de terminal")
            }
        }
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn install_panic_hook() {
    let default_hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    );
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{Action, parse_args_from};
    use crate::keys::Preset;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn sin_argumentos_abre_un_buffer_sin_titulo() {
        assert_eq!(
            parse_args_from(args(&[])).ok(),
            Some(Action::Edit(None, None))
        );
    }

    #[test]
    fn acepta_una_ruta() {
        assert_eq!(
            parse_args_from(args(&["notas.md"])).ok(),
            Some(Action::Edit(Some(PathBuf::from("notas.md")), None))
        );
    }

    #[test]
    fn reconoce_los_modos_de_teclado() {
        assert_eq!(
            parse_args_from(args(&["--vim"])).ok(),
            Some(Action::Edit(None, Some(Preset::Vim)))
        );
        assert_eq!(
            parse_args_from(args(&["-vim", "notas.md"])).ok(),
            Some(Action::Edit(
                Some(PathBuf::from("notas.md")),
                Some(Preset::Vim)
            ))
        );
        assert_eq!(
            parse_args_from(args(&["--keymap=default"])).ok(),
            Some(Action::Edit(None, Some(Preset::Default)))
        );
        assert_eq!(
            parse_args_from(args(&["--keymap", "vim"])).ok(),
            Some(Action::Edit(None, Some(Preset::Vim)))
        );
        assert!(parse_args_from(args(&["--keymap", "emacs"])).is_err());
        assert!(parse_args_from(args(&["--emacs"])).is_err());
    }

    #[test]
    fn reconoce_la_ayuda() {
        assert_eq!(parse_args_from(args(&["--help"])).ok(), Some(Action::Help));
        assert_eq!(parse_args_from(args(&["-h"])).ok(), Some(Action::Help));
    }

    #[test]
    fn reconoce_la_version() {
        assert_eq!(
            parse_args_from(args(&["--version"])).ok(),
            Some(Action::Version)
        );
        assert_eq!(parse_args_from(args(&["-V"])).ok(), Some(Action::Version));
    }

    #[test]
    fn rechaza_mas_de_una_ruta() {
        assert!(parse_args_from(args(&["uno.md", "dos.md"])).is_err());
    }

    #[test]
    fn rechaza_una_ruta_vacia() {
        assert!(parse_args_from(args(&[""])).is_err());
    }
}
