<p align="center">
  <img src="assets/PapiroMd.png" alt="Papiro" width="288">
</p>

# Papiro

[![crates.io](https://img.shields.io/crates/v/ppr.svg)](https://crates.io/crates/ppr)
[![CI](https://github.com/Ricardo-Castro-Gastelum/papiro/actions/workflows/ci.yml/badge.svg)](https://github.com/Ricardo-Castro-Gastelum/papiro/actions/workflows/ci.yml)

[English](README.md) · Español

Un editor y lector de archivos Markdown para la terminal, escrito en Rust.

Papiro se integra con la terminal en lugar de sentirse como una aplicación
aparte: usa el fondo y los colores por defecto, no dibuja marcos y no muestra
pantallas de bienvenida. Al ejecutar `ppr archivo.md`, el archivo se abre
directamente.

> Estado: publicado en crates.io como el paquete `ppr`. Incluye edición y
> guardado, lectura con el Markdown renderizado, deshacer y rehacer, ajuste de
> líneas al ancho de la terminal y búsqueda de texto.

## Requisitos

- Rust 1.88 o posterior.
- Una terminal con soporte de color verdadero (recomendado).

## Compilación

```bash
git clone https://github.com/Ricardo-Castro-Gastelum/papiro.git
cd papiro
cargo build --release
```

El binario queda en `target/release/ppr`.

## Uso

```bash
ppr archivo.md
```

Para ver la ayuda o la versión sin abrir ningún archivo:

```bash
ppr --help
ppr --version
```

Si el archivo no existe, se abre un búfer vacío con esa ruta y se crea al
guardarlo por primera vez. Si se ejecuta `ppr` sin argumentos, se abre un búfer
sin título.

Al guardar un búfer sin nombre, Papiro pide el nombre en la línea inferior.
Escribe la ruta y pulsa `Enter` para guardar, o `Esc` para cancelar.

### Atajos

| Tecla | Acción |
| --- | --- |
| Flechas | Mover el cursor en Fuente o desplazarse en Vista |
| `Ctrl+←` / `Ctrl+→` | Mover el cursor por palabras |
| Inicio / Fin | Ir al principio o al final de la línea (o del documento en Vista) |
| Re Pág / Av Pág | Desplazarse una pantalla |
| `Enter` | Insertar una línea nueva |
| `Retroceso` | Borrar el carácter anterior |
| `Supr` | Borrar el carácter siguiente |
| `Tab` | Insertar dos espacios |
| `Ctrl+Z` | Deshacer |
| `Ctrl+Y` / `Ctrl+Shift+Z` | Rehacer |
| `Ctrl+F` | Buscar texto |
| `F3` / `Shift+F3` | Ir a la coincidencia siguiente o anterior |
| `Ctrl+V` | Alternar entre Fuente y Vista |
| `Esc` | Volver de Vista a Fuente |
| `Ctrl+S` | Guardar |
| `Ctrl+Q` | Salir (pide confirmación si hay cambios sin guardar) |

`Ctrl+Q` sale de inmediato si no hay cambios pendientes. Si los hay, la línea
inferior muestra un aviso: `Ctrl+Q` otra vez descarta los cambios y sale,
`Ctrl+S` guarda y cualquier otra tecla (incluido `Esc`) cancela.

## Deshacer y rehacer

`Ctrl+Z` deshace la última edición y `Ctrl+Y` (o `Ctrl+Shift+Z`) la rehace. El
tecleo seguido y los borrados seguidos se agrupan en un solo paso; mover el
cursor, cambiar de modo o guardar cierra el grupo. Deshacer también devuelve el
cursor a donde estaba y, si el texto vuelve exactamente al estado guardado, el
punto `•` desaparece de la barra de estado.

## Ajuste de líneas

En el modo Fuente, las líneas más largas que la terminal se muestran en varias
filas sin modificar el archivo. Las flechas verticales y `Re Pág`/`Av Pág`
recorren esas filas y conservan la columna visual al pasar de una a otra.
`Inicio` y `Fin` siguen yendo al principio y al final de la línea lógica. En el
modo Vista el texto también se ajusta, pero allí solo se desplaza la lectura.

## Búsqueda

`Ctrl+F` abre el indicador de búsqueda en la línea inferior y salta a la primera
coincidencia mientras escribes, sin distinguir mayúsculas. `Enter` avanza a la
siguiente, `Shift+Enter` vuelve a la anterior y la coincidencia actual se
resalta en el texto. `Esc` cierra la búsqueda y deja el cursor en la
coincidencia encontrada.

Después de cerrar la búsqueda, `F3` repite la última búsqueda desde el cursor y
`Shift+F3` busca hacia atrás. Si no hay coincidencias, la línea inferior lo
indica. Si estabas en el modo Vista, `Ctrl+F` vuelve a Fuente para poder
recorrer las coincidencias.

## Barra de estado

La línea inferior muestra el nombre del archivo y, cuando hay cambios sin
guardar, un punto `•` a su derecha. En el modo Fuente también indica la línea y
la columna del cursor (`Ln 12, Col 5`) y recuerda los atajos. En terminales
angostas los atajos se abrevian para no ocultar el nombre del archivo.

## Modo Vista

`Ctrl+V` muestra el documento con el Markdown interpretado, sin marcas de
sintaxis. En este modo el texto es de solo lectura: las flechas, `Inicio`,
`Fin`, `Re Pág` y `Av Pág` desplazan la vista, y `Ctrl+V` o `Esc` vuelven al
modo Fuente. La línea inferior indica el modo y recuerda los atajos.

La vista reconoce:

- Encabezados, con negrita y subrayado según el nivel.
- Negrita, cursiva, tachado y `código en línea`.
- Bloques de código con sangría.
- Listas con viñetas, listas numeradas, listas anidadas y tareas.
- Citas con una barra lateral.
- Enlaces subrayados (sin mostrar la URL).
- Reglas horizontales y tablas.
- Imágenes como `[imagen: descripción]`.

El texto se ajusta automáticamente al ancho de la terminal, respetando los
acentos y los emojis compuestos. Es un ajuste visual: el archivo guardado no
cambia.

## Guardado

Papiro escribe primero en un archivo temporal situado en el mismo directorio y
después lo renombra sobre el destino. Si algo falla durante la escritura, el
archivo original no se daña. Al sobrescribir, se conservan los permisos del
archivo existente; los archivos nuevos respetan el umask del sistema.

El editor normaliza los finales de línea a LF y guarda siempre en UTF-8.

## Instalación

Desde crates.io:

```bash
cargo install ppr
```

Para instalarlo desde una copia local del código:

```bash
cargo install --path .
```

Copia el binario a `~/.local/bin` si prefieres instalarlo sin Cargo:

```bash
mkdir -p ~/.local/bin
cp target/release/ppr ~/.local/bin/
```

Asegúrate de que `~/.local/bin` esté en tu `PATH`.

## Licencia

Papiro se distribuye bajo la licencia dual MIT o Apache 2.0. Puedes elegir la
que prefieras:

- [MIT](LICENSE-MIT)
- [Apache 2.0](LICENSE-APACHE)
