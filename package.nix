{
  lib,
  stdenv,
  fetchurl,
  autoPatchelfHook,
  makeWrapper,
  copyDesktopItems,
  makeDesktopItem,
  fontconfig,
  wayland,
  libxkbcommon,
  libX11,
  libXcursor,
  libXrandr,
  libXi,
  libGL,
  vulkan-loader,
}:

stdenv.mkDerivation rec {
  pname = "favnyr";
  version = "0.3.4";

  src = fetchurl {
    url = "https://github.com/GlitchAwakened/Favnyr/releases/download/v${version}/favnyr-linux-x86_64.tar.gz";
    hash = "sha256-ft0MkM4Dz1Mq+brX2SCHagsAiISuYIuF0v2u7lAhfkk=";
  };

  nativeBuildInputs = [
    autoPatchelfHook
    makeWrapper
    copyDesktopItems
  ];

  buildInputs = [
    stdenv.cc.cc.lib
    fontconfig
    wayland
    libxkbcommon
    libX11
    libXcursor
    libXrandr
    libXi
    libGL
    vulkan-loader
  ];

  sourceRoot = ".";

  desktopItems = [
    (makeDesktopItem {
      name = "favnyr";
      exec = "favnyr";
      icon = "favnyr";
      comment = "A modern and lightweight file manager";
      desktopName = "Favnyr";
      genericName = "File Manager";
      categories = [
        "System"
        "FileManager"
        "Utility"
      ];
    })
  ];

  installPhase = ''
    runHook preInstall

    install -Dm755 favnyr $out/bin/favnyr
    install -Dm644 favnyr.svg $out/share/icons/hicolor/scalable/apps/favnyr.svg

    runHook postInstall
  '';

  postFixup = ''
    wrapProgram $out/bin/favnyr \
      --prefix LD_LIBRARY_PATH : ${
        lib.makeLibraryPath [
          wayland
          libxkbcommon
          libGL
          vulkan-loader
          libX11
          libXcursor
          libXrandr
          libXi
          fontconfig
        ]
      }
  '';

  meta = with lib; {
    description = "A modern and lightweight file manager";
    homepage = "https://github.com/GlitchAwakened/Favnyr";
    license = licenses.gpl3Plus;
    mainProgram = "favnyr";
    platforms = [ "x86_64-linux" ];
  };
}