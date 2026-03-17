{
  description = "FastTrackStudio — reproducible music production & testing environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    let
      # ── Configuration presets ──────────────────────────────────
      #
      # mkFtsEnv takes an option set and returns packages/shells.
      # Consumers can call self.lib.mkFtsEnv { plugins.lv2 = true; }
      # or use the built-in presets exposed as packages.

      defaultConfig = {
        # Audio plugins to include in the FHS environment
        plugins = {
          lv2 = false;
          vst = false;
          vst3 = false;
          clap = false;
          ladspa = false;
        };

        # Audio backend libraries
        audio = {
          pipewire = true;
          pulseaudio = true;
          alsa = true;
          jack = true;
        };

        # Media codec libraries
        codecs = {
          ffmpeg = false;
          lame = true;
          vorbis = true;
          ogg = true;
          flac = true;
          opus = true;
          sndfile = true;
        };

        # Developer / debugging tools in the dev shell
        dev = {
          rust = true;
          debug = false; # strace, gdb
        };

        # Headless display
        headless = {
          enable = true;
          resolution = "1920x1080x24";
          display = ":99";
        };
      };

      # Merge user config over defaults (one level deep per section)
      mergeConfig =
        user:
        let
          merge =
            section:
            if builtins.hasAttr section user then
              (defaultConfig.${section} // user.${section})
            else
              defaultConfig.${section};
        in
        {
          plugins = merge "plugins";
          audio = merge "audio";
          codecs = merge "codecs";
          dev = merge "dev";
          headless = merge "headless";
        };
    in
    {
      # ── Library: build a custom FTS environment ──────────────
      lib.mkFtsEnv =
        userConfig: system:
        let
          cfg = mergeConfig userConfig;
          pkgs = import nixpkgs {
            inherit system;
            config.allowUnfreePredicate =
              pkg:
              builtins.elem (pkgs.lib.getName pkg) [
                "reaper"
              ];
          };

          reaper = pkgs.reaper;

          # ── Conditional dependency lists ────────────────────

          graphicsLibs = with pkgs; [
            libx11
            libxi
            libxext
            libxrandr
            libxcursor
            libxinerama
            libxcomposite
            libxdamage
            libxfixes
            libxrender
            libxtst
            libxcb
            gtk3
            gdk-pixbuf
            glib
            pango
            cairo
            atk
            libGL
            libGLU
            mesa
          ];

          audioLibs =
            with pkgs;
            [ ]
            ++ pkgs.lib.optionals cfg.audio.alsa [ alsa-lib ]
            ++ pkgs.lib.optionals cfg.audio.pipewire [ pipewire ]
            ++ pkgs.lib.optionals cfg.audio.jack [ pipewire.jack ]
            ++ pkgs.lib.optionals cfg.audio.pulseaudio [ pulseaudio ];

          codecLibs =
            with pkgs;
            [ ]
            ++ pkgs.lib.optionals cfg.codecs.ffmpeg [ ffmpeg ]
            ++ pkgs.lib.optionals cfg.codecs.lame [ lame ]
            ++ pkgs.lib.optionals cfg.codecs.vorbis [ libvorbis ]
            ++ pkgs.lib.optionals cfg.codecs.ogg [ libogg ]
            ++ pkgs.lib.optionals cfg.codecs.flac [ flac ]
            ++ pkgs.lib.optionals cfg.codecs.opus [ libopus ]
            ++ pkgs.lib.optionals cfg.codecs.sndfile [ libsndfile ];

          pluginPackages =
            with pkgs;
            [ ]
            ++ pkgs.lib.optionals cfg.plugins.lv2 [
              calf
              lsp-plugins
              x42-plugins
              zam-plugins
            ]
            ++ pkgs.lib.optionals cfg.plugins.ladspa [
              ladspa-sdk
            ]
            ++ pkgs.lib.optionals cfg.plugins.clap [
              # CLAP plugins — add as they become available in nixpkgs
            ];

          miscLibs = with pkgs; [
            fontconfig
            freetype
            libsm
            libice
            dbus
            zlib
            stdenv.cc.cc.lib
          ];

          fhsLibs = graphicsLibs ++ audioLibs ++ codecLibs ++ miscLibs;

          fhsPackages =
            with pkgs;
            [
              reaper
              coreutils
              bash
              procps
              which
              gnugrep
              findutils
            ]
            ++ pkgs.lib.optionals cfg.headless.enable [
              xvfb-run
              xdotool
              xauth
              xset
            ]
            ++ pluginPackages;

          # ── Derivations ─────────────────────────────────────

          reaper-fhs = pkgs.buildFHSEnv {
            name = "reaper-env";
            targetPkgs = _: fhsPackages;
            multiPkgs = _: fhsLibs;
            profile = ''
              export REAPER_BIN="${reaper}/opt/REAPER/reaper"
              export REAPER_RESOURCE_DIR="${reaper}/opt/REAPER"
              export FTS_REAPER_EXECUTABLE="${reaper}/opt/REAPER/reaper"
              export FTS_REAPER_RESOURCES="${reaper}/opt/REAPER"
            '';
            runScript = "bash";
          };

          reaper-headless = pkgs.writeShellScriptBin "reaper-headless" ''
            set -euo pipefail

            DISPLAY="${cfg.headless.display}"
            export DISPLAY

            REAPER_HOME="''${FTS_HOME:-$HOME/.config/fts-test}"
            mkdir -p "$REAPER_HOME"

            echo "[fts] Starting Xvfb on $DISPLAY..."
            ${pkgs.xorg-server}/bin/Xvfb "$DISPLAY" -screen 0 ${cfg.headless.resolution} -nolisten tcp &
            XVFB_PID=$!
            for i in $(seq 1 20); do
              if ${pkgs.xset}/bin/xset q &>/dev/null 2>&1; then
                break
              fi
              sleep 0.1
            done

            cleanup() {
              echo "[fts] Cleaning up..."
              kill "$XVFB_PID" 2>/dev/null || true
              pkill -f "reaper.*-newinst" 2>/dev/null || true
            }
            trap cleanup EXIT

            echo "[fts] Xvfb ready"
            echo "[fts] FTS_REAPER_EXECUTABLE=${reaper}/opt/REAPER/reaper"
            echo "[fts] FTS_REAPER_RESOURCES=${reaper}/opt/REAPER"

            if [ $# -gt 0 ]; then
              exec "$@"
            else
              echo "[fts] No command given — dropping into shell."
              exec bash
            fi
          '';

          reaper-test-env = pkgs.writeShellScriptBin "fts-test" ''
            exec ${reaper-fhs}/bin/reaper-env ${reaper-headless}/bin/reaper-headless "$@"
          '';

          reaper-gui = pkgs.writeShellScriptBin "fts-gui" ''
            exec ${reaper-fhs}/bin/reaper-env ${reaper}/opt/REAPER/reaper "$@"
          '';

          devShell = pkgs.mkShell {
            name = "fts-dev";
            packages =
              with pkgs;
              [
                reaper-test-env
                reaper-gui
                reaper-fhs
                pkg-config
                openssl
              ]
              ++ pkgs.lib.optionals cfg.dev.rust [ pkgs.rustup ]
              ++ pkgs.lib.optionals cfg.dev.debug [
                pkgs.strace
                pkgs.gdb
              ];

            shellHook = ''
              export FTS_REAPER_EXECUTABLE="${reaper}/opt/REAPER/reaper"
              export FTS_REAPER_RESOURCES="${reaper}/opt/REAPER"
              echo ""
              echo "  fts-flake dev shell"
              echo "  ────────────────────────────────────────"
              echo "  fts-test [cmd]  — headless FHS env (CI-ready)"
              echo "  fts-gui         — launch REAPER with GUI"
              echo "  reaper-env      — drop into bare FHS shell"
              echo ""
              echo "  REAPER: ${reaper}/opt/REAPER/reaper"
              echo ""
            '';
          };
        in
        {
          packages = {
            inherit reaper-fhs reaper-headless;
            fts-test = reaper-test-env;
            fts-gui = reaper-gui;
          };
          inherit devShell;
        };

      # ── Presets ──────────────────────────────────────────────
      #
      # These are the ready-to-use configurations exposed as
      # standard flake outputs. Consumers can also call
      # self.lib.mkFtsEnv with a custom config.

      presets = {
        # CI: minimal, no plugins, no debug tools, headless only
        ci = {
          plugins = {
            lv2 = false;
            vst = false;
            vst3 = false;
            clap = false;
            ladspa = false;
          };
          audio = {
            pipewire = false;
            pulseaudio = false;
            alsa = true;
            jack = false;
          };
          codecs = {
            ffmpeg = false;
            lame = false;
            vorbis = false;
            ogg = false;
            flac = false;
            opus = false;
            sndfile = true;
          };
          dev = {
            rust = true;
            debug = false;
          };
          headless = {
            enable = true;
            resolution = "1280x720x16";
            display = ":99";
          };
        };

        # Dev: full environment with plugins and debug tools
        dev = {
          plugins = {
            lv2 = true;
            vst = false;
            vst3 = false;
            clap = true;
            ladspa = false;
          };
          audio = {
            pipewire = true;
            pulseaudio = true;
            alsa = true;
            jack = true;
          };
          codecs = {
            ffmpeg = true;
            lame = true;
            vorbis = true;
            ogg = true;
            flac = true;
            opus = true;
            sndfile = true;
          };
          dev = {
            rust = true;
            debug = true;
          };
          headless = {
            enable = true;
            resolution = "1920x1080x24";
            display = ":99";
          };
        };

        # Production: full plugins, all codecs, no debug
        full = {
          plugins = {
            lv2 = true;
            vst = false;
            vst3 = false;
            clap = true;
            ladspa = true;
          };
          audio = {
            pipewire = true;
            pulseaudio = true;
            alsa = true;
            jack = true;
          };
          codecs = {
            ffmpeg = true;
            lame = true;
            vorbis = true;
            ogg = true;
            flac = true;
            opus = true;
            sndfile = true;
          };
          dev = {
            rust = false;
            debug = false;
          };
          headless = {
            enable = false;
            resolution = "1920x1080x24";
            display = ":99";
          };
        };
      };
    }
    // flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        defaultEnv = self.lib.mkFtsEnv { } system;
        ciEnv = self.lib.mkFtsEnv self.presets.ci system;
        devEnv = self.lib.mkFtsEnv self.presets.dev system;
        fullEnv = self.lib.mkFtsEnv self.presets.full system;

        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfreePredicate =
            pkg:
            builtins.elem (pkgs.lib.getName pkg) [
              "reaper"
            ];
        };
      in
      {
        packages = {
          default = defaultEnv.packages.fts-test;

          # Preset packages
          fts-test = defaultEnv.packages.fts-test;
          fts-test-ci = ciEnv.packages.fts-test;
          fts-test-dev = devEnv.packages.fts-test;
          fts-gui = defaultEnv.packages.fts-gui;
          fts-gui-dev = devEnv.packages.fts-gui;
          reaper-fhs = defaultEnv.packages.reaper-fhs;
        };

        devShells = {
          default = devEnv.devShell;
          ci = ciEnv.devShell;
          minimal = defaultEnv.devShell;
          full = fullEnv.devShell;
        };

        checks.reaper-starts = pkgs.runCommand "reaper-starts" { } ''
          test -x ${pkgs.reaper}/opt/REAPER/reaper
          echo "REAPER binary OK" > $out
        '';
      }
    );
}
