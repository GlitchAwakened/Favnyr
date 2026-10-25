# Favnyr

**Favnyr** : A portable file browser for Windows and Linux. Single executable, no installer, no administrator rights, no telemetry.

<p align="center">
<img width="30%" alt="favnyr" src="https://github.com/user-attachments/assets/1132cb2d-c7c2-4554-8e2d-d285b07139bd" />

<img width="60%" alt="ScreenShotFavnyr01" src="https://github.com/user-attachments/assets/086c3a57-9f72-401f-837b-40887c60919c">
</p>

https://github.com/user-attachments/assets/a59d8aac-5b9d-499d-bc37-f774ffce9066

https://github.com/user-attachments/assets/2522fd49-1803-4419-8fa6-deba85dacc20

*(It's worth watching the videos part 1 & 2 until the end. It goes well past the basics and shows most of the
gestures and shortcuts collected under [Pro tips](#pro-tips).)*

---

Favnyr came out of a simple frustration: wanting several folders open side by side, being able to come back to the same arrangement tomorrow, and not wondering what the program does when nobody is looking. It is written in Rust with [Slint](https://slint.dev), which is why it starts fast and stays small.
I am Favnyr's first user, and I use it daily literally every day.

Copy the executable where you want and run it. That is the whole installation.

**NixOS* : See the [NixOS footnote](#nixos) below.

## Private by construction

Favnyr makes no network request. There is no account, no telemetry, no update check, no crash reporting.

Its settings, favorites and workspaces are plain TOML files in the usual per-user folders. You do not have to guess where: **Settings → Storage** lists the exact paths and opens them.

Thumbnails live in a bounded memory cache for the length of the session. Nothing is written to a thumbnail database on disk.

Favnyr runs with ordinary user permissions. It installs no service and no background process. It still obeys the permissions of the files you ask it to touch, obviously.

## What it does

**Views.** Split the window horizontally or vertically, up to 32 views. Each keeps its own tabs, history, sorting and columns.

**Workspaces.** A workspace is a saved set of views and tabs. Keep one per project and restore the whole arrangement in one click, or assign it a number to load it with a keyboard shortcut. Settings provide separate save policies for the Workspaces panel and numbered shortcuts (CTRL+1, CTRl+2...). A workspace can have its own theme, independently of the global theme.

**Favorites.** Shared across every workspace, so your usual places do not depend on which layout is open.

**Terminal.** Open one in the current folder, from the right-click menu or the keyboard. **Settings > General > Terminal** chooses which one.

**Custom commands.** Run any program on the selection with your own arguments, using placeholders: `{file}`, `{files}` (the whole selection), `{dir}`, `{dirname}`, `{setname}`, `{name}`, `{stem}`, `{ext}`, `{names}`, `{uri}`. A command can be pinned to the right-click menu, and restricted to the extensions where it makes sense.

**Ready-made recipes** (ready-made commands) are proposed when available : send by email, send to a device and archive commands. Each arrives already pinned to the right-click menu and limited to the extensions where it makes sense. This is genuinely useful on Linux, where no equivalent shell integration exists.
So if 7-Zip is installed (7zz, 7z or 7za), Favnyr compresses and extracts without leaving the window :
- `Compress to…` opens a form where you can define : archive name, format (ZIP, 7z or TAR), compression level, and an optional password.
- `Extract here` and `Extract to folder` work on the archive types 7-Zip reads, RAR included.


**Keyboard.** Actions follow the usual conventions and can be rebound from Settings.

**Columns.** Show, hide, resize and reorder them per view.

**List and grid.** The display menu selects the arrangement and whether to show icons or thumbnails. Each tab remembers its layout and grid size; Ctrl+wheel adjusts the size. The grid keeps selection, file drops, folder annotations and manual ordering. In manual order, drop before or after a tile to reposition the selection, including across rows. Drop in the centre of a folder to target that folder. Settings provide defaults for new tabs, text size and spacing on both grid axes.

**Filtering.** Start typing to narrow a folder by name, or open the extension filter for something stricter.

**Recursive folder size and date.** Off by default. Turn either on and choose a depth from 1 to 16: Favnyr then shows the real size and latest modification date of a folder's contents, computed in the background. Both metrics share a single directory walk, so enabling the two costs one pass, not two.

**Files and folders.** Copy, move, link, rename, create, trash and delete, with progress reported. Several operations can run at once. Multi-selection, drag and drop between views and to other applications, and reopening closed tabs. Folders can be given a color, any item (file/folder) a short note, and both follow it when it is renamed or moved within Favnyr.

**Properties.** A built-in panel for when no desktop file manager offers one: type, location, sizes and dates, Unix permissions and ownership, and MD5, SHA-1, SHA-256 and SHA-512 checksums computed on request. Every value is selectable.

**Languages.** English, French, Spanish, German, Italian, Russian, Simplified Chinese, and Japanese.

**Themes.** Nord, Atom, Tokyo Night, Sonokai Atlantis, Catppuccin Frappé, Macchiato, Mocha, Rosé Pine, Everforest, Gruvbox Hard, Fluvy... each available in light and dark variants. You can also [create your own themes](#custom-themes).

## Thumbnails

Images are decoded in pure Rust :

`.jpg`, `.jpeg`, `.png`, `.gif`, `.webp`, `.bmp`, `.tif`, `.tiff`, `.tga`, `.hdr`, `.ico`

`.svg` is rendered by Slint's own vector engine.

PSD files (`.psd`) display Photoshop's embedded JPEG thumbnail when one is
present. This is deliberately best-effort.

Affinity Photo, Designer and Publisher files (`.afphoto`, `.afdesign`, `.afpub`
and `.af`) receive the same lightweight treatment.

MP3 and FLAC files (`.mp3`, `.flac`) display their embedded cover art. A track without embedded artwork simply keeps the normal audio icon.

3D files `.blend`, `.3mf`, `.stl`, `.glb`, `.gltf`, `.obj` thumbnails are supported. 7-Zip is only necessary for a **compressed** `.blend`.

Videos (`.mp4`, `.mkv`, `.mov`, `.avi`, `.webm`, `.wmv`, `.flv`, `.m4v`, `.mpg`, `.mpeg`, `.ts`, `.3gp`) and PDFs get a thumbnail too, through a different route on each system:

- **Windows** -> the shell's thumbnail providers, so nothing extra to install.
- **Linux** -> `ffmpeg` for video, Poppler (`pdftoppm` or `pdftocairo`) for PDF. Both optional; without them Favnyr simply shows the file-type icon.

## Pro tips

A few things that are easy to miss.

- **Type to filter.** Any printable character typed in a view filters its
  entries by name as you go.
- **Ctrl+wheel resizes the entries**, and past a size threshold they switch to
  thumbnails.
- **Shift+wheel scrolls a view horizontally** when its columns overflow.
- **Tab / Shift+Tab select the next / previous view**, wrapping at either end.
  Both shortcuts can be changed in Settings.
- **The wheel over a tab bar switches tabs**, forwards and back, wrapping around
  at the ends. It works the same on a vertical tab bar, and a horizontal
  trackpad swipe counts too.
- **The wheel still scrolls during a drag.** It scrolls whichever view sits
  under the cursor, so a folder further down the list can be reached without
  releasing the drag.
- **Hold Space while dragging one folder** and the drag opens it instead of
  moving it: drop on the edge of a view to split there, or in its middle to add
  a tab.
- **Drag modifiers.** Ctrl copies straight away, Shift moves straight away, and
  no modifier shows the Move/Copy menu.
- **Drop files on an executable** to open them with that program.
- **Double-click the gutter**, the narrow empty strip to the left of the
  entries, to select everything (similar to CTRL+A).
- **Double-click a separator** to even out the views it divides, and double-click
  it again to put the previous sizes back.
- **Middle-click** is three gestures: held down in a view it scrolls from where
  it was pressed, on a sidebar entry or a favorite it opens in a new tab, and on
  a tab it closes it.
- **Workspace shortcuts.** In the Workspaces panel (Ctrl+Q by default), each
  saved workspace has a badge for assigning a slot from 1 to 9. The matching
  shortcut (Ctrl+1 to Ctrl+9 by default) loads it directly. These shortcuts
  can be rebound, including on keyboards where the digit row needs Shift.
- **Unsaved workspace changes.** Settings → General → Workspaces has separate
  three-choice policies for loading from the panel and from numbered shortcuts:
  ask when changes are unsaved (the default), continue without saving, or save
  the current workspace first. The panel policy also applies to
  **Start new workspace**. In a confirmation, **Remember this choice** updates only the
  policy for that action's source. Cancel leaves both the workspace and the
  policy unchanged. If saving fails, the load or new workspace is stopped and
  an error is shown.

## Custom themes

Favnyr's themes are plain TOML files, so the simplest way to make a theme is to copy one that already
exists, for instance `crates/favnyr-core/src/theme/builtins/atom.toml`, and edit the colors you want to change.

Drop the file in Favnyr's themes directory, under any name you like:

- **Linux** -> `~/.local/share/favnyr/themes` (or `XDG_DATA_HOME`)
- **Windows** -> `%APPDATA%\favnyr\themes`

A theme declares **both appearances**, `light` and `dark`.
Every role you leave out is inherited from Nord, Favnyr's base theme, so a complete and valid theme can be this short:

```toml
schema_version = 1

[light.colors]
accent_background = "#1F7A54"

[dark.colors]
accent_background = "#A0D6BC"
```

`crates/favnyr-core/src/theme/builtins/nord.toml` is the exhaustive reference:
it carries all 56 color roles in both appearances, with the names to use. Colors are written `#RRGGBB`, and
`#RRGGBBAA` is accepted only for `track`, `disk_normal`, `disk_warning`,
`disk_critical` and `modal_overlay`, the roles that are drawn over something
else. Each appearance also takes an optional `[<appearance>.opacity]` table
with `hidden` and `age`, two numbers between zero and one that dim hidden items
and the background of file-age badges.


## About AI-assisted development

Because of the project’s complexity, AI has been used as a development assistant.
Nothing was accepted because it compiled. Every change was carefully tried by hand on both Windows and Linux, lived with, and reworked until it held up in the awkward cases as well as the obvious ones. That patient testing and finishing took far longer than the writing. 

## Building

Needs Rust 1.92 or newer and a C compiler. The first release build takes several minutes: it is optimised with link-time optimisation.

### Windows

Install Rust from <https://rust-lang.org/tools/install/> (the x64 `rustup-init.exe`), then the Visual Studio Build Tools with the **Desktop development with C++** workload, which provides MSVC and the Windows SDK:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Video and PDF thumbnails use Windows providers.

### Linux

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then the build dependencies:

- Debian / Ubuntu / Mint -> `sudo apt install build-essential pkg-config libfontconfig1-dev`
- Fedora -> `sudo dnf install gcc pkgconf-pkg-config fontconfig-devel`
- Arch -> `sudo pacman -S --needed base-devel fontconfig`

Optionally install `ffmpeg` for video thumbnails, `poppler-utils` for PDF
thumbnails, and a recent 7-Zip (`7zz` or `7z`) for embedded 3D thumbnails and
the advanced compression and extraction recipes (ready-made commands).

### Build and run

```sh
cargo run --release --bin favnyr
```

The executable lands in `target/release/` (`favnyr` on Linux, `favnyr.exe` on Windows). Copy it anywhere. `cargo clean` reclaims the build directory, which is large.

### NixOS

#### Run directly

```bash
nix run 'github:GlitchAwakened/Favnyr?dir=packaging/nix'
```

#### Install via Profile

```bash
nix profile install 'github:GlitchAwakened/Favnyr?dir=packaging/nix'
```

#### Install via Flake

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    favnyr.url = "github:GlitchAwakened/Favnyr?dir=packaging/nix";
  };

  outputs = { self, nixpkgs, favnyr, ... }: {
    nixosConfigurations.myhostname = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            favnyr.packages.${pkgs.system}.default
          ];
        })
      ];
    };
  };
}
```

## License

GNU General Public License v3.0 or later.
