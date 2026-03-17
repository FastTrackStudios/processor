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
        # REAPER extensions (from nixpkgs)
        extensions = {
          sws = true;
          reapack = true;
        };

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
          extensions = merge "extensions";
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
          sws = pkgs.reaper-sws-extension;
          reapack = pkgs.reaper-reapack-extension;

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

          extensionPackages =
            [ ]
            ++ pkgs.lib.optionals cfg.extensions.sws [ sws ]
            ++ pkgs.lib.optionals cfg.extensions.reapack [ reapack ];

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
            ++ extensionPackages
            ++ pkgs.lib.optionals cfg.headless.enable [
              xvfb-run
              xdotool
              xauth
              xset
            ]
            ++ pluginPackages;

          # ── Extension setup script ──────────────────────────
          # Symlinks SWS/ReaPack .so files into REAPER's UserPlugins
          # directory, matching the pattern from the system flake.

          extensionSetup =
            let
              swsSetup = pkgs.lib.optionalString cfg.extensions.sws ''
                ln -sf "${sws}/UserPlugins/reaper_sws-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                ln -sf "${sws}/Scripts/sws_python.py" "$REAPER_CONFIG/Scripts/"
                ln -sf "${sws}/Scripts/sws_python64.py" "$REAPER_CONFIG/Scripts/"
                echo "[fts] SWS extension linked"
              '';
              reapackSetup = pkgs.lib.optionalString cfg.extensions.reapack ''
                ln -sf "${reapack}/UserPlugins/reaper_reapack-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                echo "[fts] ReaPack extension linked"
              '';
            in
            ''
              REAPER_CONFIG="''${HOME}/.config/REAPER"
              mkdir -p "$REAPER_CONFIG/UserPlugins" "$REAPER_CONFIG/Scripts"
              ${swsSetup}
              ${reapackSetup}
            '';

          # ── Derivations ─────────────────────────────────────

          reaper-fhs = pkgs.buildFHSEnv {
            name = "reaper-env";
            targetPkgs = _: fhsPackages;
            multiPkgs = _: fhsLibs;
            profile = ''
              export REAPER_BIN="${reaper}/bin/reaper"
              export REAPER_RESOURCE_DIR="${reaper}/opt/REAPER"
              export FTS_REAPER_EXECUTABLE="${reaper}/bin/reaper"
              export FTS_REAPER_RESOURCES="${reaper}/opt/REAPER"

              # Set up plugin search paths
              export LV2_PATH="''${LV2_PATH:+$LV2_PATH:}/usr/lib/lv2"
              export CLAP_PATH="''${CLAP_PATH:+$CLAP_PATH:}/usr/lib/clap"
              export VST_PATH="''${VST_PATH:+$VST_PATH:}/usr/lib/vst"
              export VST3_PATH="''${VST3_PATH:+$VST3_PATH:}/usr/lib/vst3"
              export LADSPA_PATH="''${LADSPA_PATH:+$LADSPA_PATH:}/usr/lib/ladspa"
              export DSSI_PATH="''${DSSI_PATH:+$DSSI_PATH:}/usr/lib/dssi"
            '';
            runScript = "bash";
          };

          reaper-headless = pkgs.writeShellScriptBin "reaper-headless" ''
            set -euo pipefail

            # Find a free display number (avoids conflicts with existing X sessions)
            PREFERRED="${cfg.headless.display}"
            DISPLAY="$PREFERRED"
            for n in $(seq ''${PREFERRED#:} 120); do
              if [ ! -e "/tmp/.X''${n}-lock" ]; then
                DISPLAY=":$n"
                break
              fi
            done
            export DISPLAY

            REAPER_HOME="''${FTS_HOME:-$HOME/.config/fts-test}"
            mkdir -p "$REAPER_HOME"

            # Link extensions into REAPER config
            ${extensionSetup}

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

            echo "[fts] Xvfb ready on $DISPLAY"
            echo "[fts] FTS_REAPER_EXECUTABLE=${reaper}/bin/reaper"
            echo "[fts] FTS_REAPER_RESOURCES=${reaper}/opt/REAPER"

            # Override to use the nixpkgs wrapper (sets LD_LIBRARY_PATH correctly)
            export FTS_REAPER_EXECUTABLE="${reaper}/bin/reaper"

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
            # Link extensions before launching GUI
            ${extensionSetup}
            exec ${reaper-fhs}/bin/reaper-env ${reaper}/bin/reaper "$@"
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
              export FTS_REAPER_EXECUTABLE="${reaper}/bin/reaper"
              export FTS_REAPER_RESOURCES="${reaper}/opt/REAPER"
              echo ""
              echo "  fts-flake dev shell"
              echo "  ────────────────────────────────────────"
              echo "  fts-test [cmd]  — headless FHS env (CI-ready)"
              echo "  fts-gui         — launch REAPER with GUI"
              echo "  reaper-env      — drop into bare FHS shell"
              echo ""
              echo "  REAPER:  ${reaper}/bin/reaper"
              echo "  SWS:     ${if cfg.extensions.sws then "enabled" else "disabled"}"
              echo "  ReaPack: ${if cfg.extensions.reapack then "enabled" else "disabled"}"
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
        # CI: minimal — no plugins, no extensions, headless only
        ci = {
          extensions = {
            sws = false;
            reapack = false;
          };
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

        # Dev: full environment with extensions, plugins, and debug tools
        dev = {
          extensions = {
            sws = true;
            reapack = true;
          };
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

        # Production: full plugins + extensions, all codecs, no debug
        full = {
          extensions = {
            sws = true;
            reapack = true;
          };
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
