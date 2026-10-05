# Favnyr
<p align="center">
<img width="30%" alt="favnyr" src="https://github.com/user-attachments/assets/1132cb2d-c7c2-4554-8e2d-d285b07139bd" />

<img width="60%" alt="ScreenShotFavnyr01" src="https://github.com/user-attachments/assets/086c3a57-9f72-401f-837b-40887c60919c">
</p>

https://github.com/user-attachments/assets/78f459cc-ada4-4f07-9ed5-2132f3062fa6

A portable file browser for Windows and Linux. Single executable, no installer, no administrator rights, no telemetry.

Favnyr came out of a simple frustration: wanting several folders open side by side, being able to come back to the same arrangement tomorrow, and not wondering what the program does when nobody is looking. It is written in Rust with [Slint](https://slint.dev), which is why it starts fast and stays small.
I am Favnyr's first user, and I use it daily literally every day.

Copy the executable where you want and run it. That is the whole installation.

**Special case** : on NixOS there is no `/usr/lib`, so the binary finds neither fontconfig nor the OpenGL driver on its own. Run it through an FHS environment instead:
`steam-run ./favnyr`


## Private by construction

Favnyr makes no network request. There is no account, no telemetry, no update check, no crash reporting.

Its settings, favorites and workspaces are plain TOML files in the usual per-user folders. You do not have to guess where: **Settings → Storage locations** lists the exact paths and opens them.

Thumbnails live in a bounded memory cache for the length of the session. Nothing is written to a thumbnail database on disk.

Favnyr runs with ordinary user permissions. It installs no service and no background process. It still obeys the permissions of the files you ask it to touch, obviously.

## What it does

**Panels.** Split the window horizontally or vertically, up to 16 panels. Each keeps its own tabs, history, sorting and columns.

**Workspaces.** A workspace is a saved set of panels and tabs. Keep one per project and restore the whole arrangement in one click.

**Favorites.** Shared across every workspace, so your usual places do not depend on which layout is open.

**Custom commands.** Run any program on the selection with your own arguments, using placeholders: `{file}`, `{files}` (the whole selection), `{dir}`, `{dirname}`, `{setname}`, `{name}`, `{stem}`, `{ext}`, `{names}`, `{uri}`. A command can be pinned to the right-click menu, and restricted to the extensions where it makes sense.

**Ready-made recipes** (ready-made commands) are proposed when available : send by email, send to a device and archive commands. Each arrives already pinned to the right-click menu and limited to the extensions where it makes sense. This is genuinely useful on Linux, where no equivalent shell integration exists.
So if 7-Zip is installed (7zz, 7z or 7za), Favnyr compresses and extracts without leaving the window :
- `Compress to…` opens a form where you can define : archive name, format (ZIP, 7z or TAR), compression level, and an optional password.
- `Extract here` and `Extract to folder` work on the archive types 7-Zip reads, RAR included.


**Keyboard.** 39 actions follow the usual conventions and can all be rebound from Settings.

**Columns.** Show, hide, resize and reorder them per panel.

**Filtering.** Start typing to narrow a folder by name, or open the extension filter for something stricter.

**Recursive folder size and date.** Off by default. Turn either on and choose a depth from 1 to 8: Favnyr then shows the real size and latest modification date of a folder's contents, computed in the background. Both metrics share a single directory walk, so enabling the two costs one pass, not two.

**Files and folders.** Copy, move, link, rename, create, trash and delete, with progress reported. Several operations can run at once. Multi-selection, drag and drop between panels and to other applications, and reopening closed tabs. Folders can be given a color, any item (file/folder) a short note, and both follow it when it is renamed or moved within Favnyr.

**Interface.** English, French, Spanish, German and Italian. Light and dark themes. Adjustable scaling.

## Previews

Images are decoded in pure Rust, no C library and no network:

`.jpg` `.jpeg` · `.png` · `.gif` · `.webp` · `.bmp` · `.tif` `.tiff` · `.tga` · `.hdr` · `.ico`

`.svg` is rendered by Slint's own vector engine.

PSD files (`.psd`) display Photoshop's embedded JPEG thumbnail when one is
present. This is deliberately best-effort.

Affinity Photo, Designer and Publisher files (`.afphoto`, `.afdesign`, `.afpub`
and `.af`) receive the same lightweight treatment.

MP3 and FLAC files (`.mp3`, `.flac`) display their embedded cover art. A track without embedded artwork simply keeps the normal audio icon.

Videos (`.mp4` `.mkv` `.mov` `.avi` `.webm` `.wmv` `.flv` `.m4v` `.mpg` `.mpeg` `.ts` `.3gp`) and PDFs get a preview too, through a different route on each system:

- **Windows** -> the shell's thumbnail providers, so nothing extra to install.
- **Linux** -> `ffmpeg` for video, Poppler (`pdftoppm` or `pdftocairo`) for PDF. Both optional; without them Favnyr simply shows the file-type icon.


## About AI-assisted development

Favnyr is built with AI assistance. Saying so plainly seems better than leaving it to be discovered.

Nothing was accepted because it compiled. Every change was carefully tried by hand on both Windows and Linux, lived with, and reworked until it held up in the awkward cases as well as the obvious ones. That patient testing and finishing took far longer than the writing. 

## Building

Needs Rust 1.88 or newer and a C compiler. The first release build takes several minutes: it is optimised with link-time optimisation.

### Windows

Install Rust from <https://rust-lang.org/tools/install/> (the x64 `rustup-init.exe`), then the Visual Studio Build Tools with the **Desktop development with C++** workload, which provides MSVC and the Windows SDK:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Nothing else is needed : previews go through the system.

### Linux

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then the build dependencies:

- Debian / Ubuntu / Mint -> `sudo apt install build-essential pkg-config libfontconfig1-dev`
- Fedora -> `sudo dnf install gcc pkgconf-pkg-config fontconfig-devel`
- Arch -> `sudo pacman -S --needed base-devel fontconfig`

Optionally install `ffmpeg` for video previews, `poppler-utils` for PDF
previews, and 7-Zip (`7zz`, `7z`, or `7za`) for the advanced compression and
extraction recipes (ready-made commands).

### Build and run

```sh
cargo run --release --bin favnyr
```

The executable lands in `target/release/` (`favnyr` on Linux, `favnyr.exe` on Windows). Copy it anywhere. `cargo clean` reclaims the build directory, which is large.

## NixOS

### Run directly

```bash
nix run github:GlitchAwakened/Favnyr
```

### Install via Profile

```bash
nix profile install github:GlitchAwakened/Favnyr
```

### Install via Flake

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    favnyr.url = "github:GlitchAwakened/Favnyr"
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
