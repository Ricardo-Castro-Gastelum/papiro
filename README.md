<p align="center">
  <img src="assets/PapiroMd.png" alt="Papiro" width="288">
</p>

# Papiro

[![crates.io](https://img.shields.io/crates/v/ppr.svg)](https://crates.io/crates/ppr)
[![CI](https://github.com/Ricardo-Castro-Gastelum/papiro/actions/workflows/ci.yml/badge.svg)](https://github.com/Ricardo-Castro-Gastelum/papiro/actions/workflows/ci.yml)

English · [Español](README.es.md) · [Website](https://ricardo-castro-gastelum.github.io/papiro-web/)

A Markdown editor and reader for the terminal, written in Rust.

Papiro is designed to blend into your terminal instead of feeling like a separate
application: it uses the terminal's default background and colors, draws no
frames, and shows no welcome screen. Run `ppr file.md` and the file opens
directly.

> Status: published on crates.io as the `ppr` package. It includes editing and
> saving, a rendered Markdown view, undo/redo, line wrapping to the terminal
> width, and text search.
>
> Note: the in-app interface and `--help` text are currently in Spanish.

## Requirements

- Rust 1.88 or newer.
- A terminal with true color support (recommended).

## Build

```bash
git clone https://github.com/Ricardo-Castro-Gastelum/papiro.git
cd papiro
cargo build --release
```

The binary is written to `target/release/ppr`.

## Usage

```bash
ppr file.md
```

To show help or the version without opening a file:

```bash
ppr --help
ppr --version
```

If the file does not exist, Papiro opens an empty buffer with that path and
creates the file the first time you save. Running `ppr` with no arguments opens
an untitled buffer.

When saving an untitled buffer, Papiro asks for the filename in the bottom line.
Type the path and press `Enter` to save, or `Esc` to cancel.

### Keybindings

| Key | Action |
| --- | --- |
| Arrow keys | Move the cursor in Source mode or scroll in View mode |
| `Ctrl+←` / `Ctrl+→` | Move the cursor by words |
| Home / End | Go to the start or end of the line (or the document in View mode) |
| Page Up / Page Down | Scroll one screen |
| `Enter` | Insert a new line |
| `Backspace` | Delete the previous character |
| `Delete` | Delete the next character |
| `Tab` | Insert two spaces |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` / `Ctrl+Shift+Z` | Redo |
| `Ctrl+F` | Search text |
| `F3` / `Shift+F3` | Go to the next or previous match |
| `Ctrl+V` | Toggle between Source and View |
| `Esc` | Return from View to Source |
| `Ctrl+S` | Save |
| `Ctrl+Q` | Quit (asks for confirmation if there are unsaved changes) |

`Ctrl+Q` quits immediately when there are no pending changes. Otherwise, the
bottom line shows a warning: `Ctrl+Q` again discards the changes and quits,
`Ctrl+S` saves, and any other key (including `Esc`) cancels.

## Undo and redo

`Ctrl+Z` undoes the last edit and `Ctrl+Y` (or `Ctrl+Shift+Z`) redoes it.
Consecutive typing and consecutive deletions are grouped into a single step;
moving the cursor, switching modes, or saving closes the group. Undo also
returns the cursor to its previous position, and if the text returns exactly to
the saved state, the `•` dot disappears from the status bar.

## Line wrapping

In Source mode, lines longer than the terminal are shown across several rows
without modifying the file. The vertical arrow keys and `Page Up`/`Page Down`
move through those rows and keep the visual column when moving from one to
another. `Home` and `End` still go to the start and end of the logical line. In
View mode the text also wraps, but there it only scrolls the reading.

## Search

`Ctrl+F` opens the search prompt on the bottom line and jumps to the first match
as you type, case-insensitively. `Enter` moves to the next match, `Shift+Enter`
to the previous one, and the current match is highlighted in the text. `Esc`
closes the search and leaves the cursor on the match.

After closing the search, `F3` repeats the last search from the cursor and
`Shift+F3` searches backwards. If there are no matches, the bottom line says so.
If you were in View mode, `Ctrl+F` switches back to Source so you can move
through the matches.

## Status bar

The bottom line shows the filename and, when there are unsaved changes, a `•`
dot to its right. In Source mode it also shows the cursor line and column
(`Ln 12, Col 5`) and reminds you of the shortcuts. On narrow terminals, the
shortcuts are abbreviated so they do not hide the filename.

## Colors

Papiro uses the 16 ANSI colors defined by your terminal theme. Titles, links,
code, quotes, and Markdown markers in both modes take the colors of the theme
you already use: change the terminal theme and Papiro follows it. There is no
color configuration file and no RGB values. Background colors, borders, and the
dim shortcuts remain untouched.

To run without colors, set `NO_COLOR`:

```bash
NO_COLOR=1 ppr file.md
```

## View mode

`Ctrl+V` shows the document with the Markdown interpreted, without syntax marks.
In this mode the text is read-only: the arrow keys, `Home`, `End`, `Page Up`, and
`Page Down` scroll the view, and `Ctrl+V` or `Esc` return to Source mode. The
bottom line shows the mode and reminds you of the shortcuts.

The view supports:

- Headings, with bold and underline depending on the level.
- Bold, italic, strikethrough, and `inline code`.
- Indented code blocks.
- Bulleted lists, numbered lists, nested lists, and tasks.
- Blockquotes with a sidebar.
- Underlined links (the URL is not shown).
- Horizontal rules and tables.
- Images as `[image: description]`.

Text wraps automatically to the terminal width, preserving accents and composed
emojis. This is a visual wrap: the saved file does not change.

## Saving

Papiro writes to a temporary file in the same directory first and then renames
it over the destination. If something fails during the write, the original file
is not damaged. When overwriting, the existing file's permissions are preserved;
new files respect the system umask.

The editor normalizes line endings to LF and always saves as UTF-8.

## Installation

From crates.io:

```bash
cargo install ppr
```

To install from a local copy of the source:

```bash
cargo install --path .
```

Copy the binary to `~/.local/bin` if you prefer to install it without Cargo:

```bash
mkdir -p ~/.local/bin
cp target/release/ppr ~/.local/bin/
```

Make sure `~/.local/bin` is in your `PATH`.

## License

Papiro is distributed under the dual MIT or Apache 2.0 license. You may choose
either:

- [MIT](LICENSE-MIT)
- [Apache 2.0](LICENSE-APACHE)
