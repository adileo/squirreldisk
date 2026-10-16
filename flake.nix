{
  description = "SquirrelDisk disk usage analyzer";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      pname = "squirreldisk";
      version = "2.5.1";

      src = pkgs.fetchurl {
        url = "https://github.com/adileo/squirreldisk/releases/download/v${version}/SquirrelDisk-x86_64.AppImage";
        hash = "sha256-qcOtRW+zC9X8TUXmTnBKIN7ovfE7YvrZPF47ibjdsmc=";
      };

      appimageContents = pkgs.appimageTools.extract {
        inherit pname version src;
      };

      # v2.5.0 passes fontconfig's index straight to its font parser. FreeType
      # stores a variable font's named instance in the upper bits; the parser
      # only accepts the collection face index in the lower 16 bits.
      fontMatcher = pkgs.writeShellScriptBin "fc-match" ''
        if [[ "$#" -eq 3 && "$1" == -f && "$2" == $'%{file}\n%{index}' ]]; then
          match="$(${pkgs.lib.getExe' pkgs.fontconfig "fc-match"} "$@")" || exit "$?"
          fontFile="''${match%$'\n'*}"
          fontIndex="''${match##*$'\n'}"
          if [[ "$fontIndex" =~ ^[0-9]+$ ]]; then
            printf '%s\n%d' "$fontFile" "$((fontIndex & 65535))"
          else
            printf '%s' "$match"
          fi
        else
          exec ${pkgs.lib.getExe' pkgs.fontconfig "fc-match"} "$@"
        fi
      '';

      squirreldisk = pkgs.stdenvNoCC.mkDerivation {
        inherit pname version;
        src = appimageContents;

        strictDeps = true;
        nativeBuildInputs = [
          pkgs.autoPatchelfHook
          pkgs.makeWrapper
        ];
        buildInputs = [
          pkgs.alsa-lib
          pkgs.stdenv.cc.cc.lib
        ];

        # Patch the native binary instead of creating an FHS environment, whose
        # bind mounts would otherwise appear as disks. These libraries use dlopen.
        runtimeDependencies = with pkgs; [
          libGL
          libx11
          libxcursor
          libxi
          libxrandr
          libxinerama
          libxkbcommon
          wayland
        ];

        dontConfigure = true;
        dontBuild = true;

        installPhase = ''
          runHook preInstall

          install -Dm755 usr/bin/squirreldisk $out/bin/squirreldisk
          install -Dm444 squirreldisk.desktop \
            $out/share/applications/squirreldisk.desktop
          substituteInPlace $out/share/applications/squirreldisk.desktop \
            --replace-fail 'Exec=squirreldisk' "Exec=$out/bin/squirreldisk"

          install -Dm444 squirreldisk.png \
            $out/share/icons/hicolor/256x256/apps/squirreldisk.png

          runHook postInstall
        '';

        postFixup = ''
          wrapProgram $out/bin/squirreldisk \
            --prefix PATH : ${fontMatcher}/bin
        '';

        meta = {
          description = "See what's using your disk space and clean it up";
          homepage = "https://github.com/adileo/squirreldisk";
          license = pkgs.lib.licenses.agpl3Only;
          mainProgram = pname;
          platforms = [ system ];
          sourceProvenance = [ pkgs.lib.sourceTypes.binaryNativeCode ];
        };
      };

      app = {
        type = "app";
        program = "${squirreldisk}/bin/squirreldisk";
        meta.description = squirreldisk.meta.description;
      };
    in
    {
      packages.${system} = {
        inherit squirreldisk;
        default = squirreldisk;
      };

      apps.${system} = {
        squirreldisk = app;
        default = app;
      };

      checks.${system}.squirreldisk = squirreldisk;
      formatter.${system} = pkgs.nixfmt;
    };
}
