{
  description = "FastTrackStudio — reproducible music production & testing environment";

  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    devenv.url = "github:cachix/devenv";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  nixConfig = {
    extra-trusted-public-keys = [
      "devenv.cachix.org-1:w1cLUi8dv3hnoSPGAuibQv+f9TZLr6cv/Hm9XgU50cw="
      "fasttrackstudio.cachix.org-1:r7v7WXBeSZ7m5meL6w0wttnvsOltRvTpXeVNItcy9f4="
    ];
    extra-substituters = [
      "https://devenv.cachix.org"
      "https://fasttrackstudio.cachix.org"
    ];
  };

  outputs =
    {
      self,
      nixpkgs,
      devenv,
      flake-utils,
      rust-overlay,
    } @ inputs:
    let
      # ── FTS environment builder ────────────────────────────────
      # Builds the REAPER FHS sandbox, headless runner, and scripts.
      # Used by both devenv modules and standalone package outputs.

      mkFtsPackages =
        { pkgs, cfg }:
        let
          reaper = pkgs.reaper;
          sws = pkgs.reaper-sws-extension;
          reapack = pkgs.reaper-reapack-extension;

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
            ++ pkgs.lib.optionals cfg.plugins.ladspa [ ladspa-sdk ]
            ++ pkgs.lib.optionals cfg.plugins.clap [ ];

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

          reaper-fhs = pkgs.buildFHSEnv {
            name = "reaper-env";
            targetPkgs = _: fhsPackages;
            multiPkgs = _: fhsLibs;
            profile = ''
              export REAPER_BIN="${reaper}/bin/reaper"
              export REAPER_RESOURCE_DIR="${reaper}/opt/REAPER"
              export FTS_REAPER_EXECUTABLE="${reaper}/bin/reaper"
              export FTS_REAPER_RESOURCES="${reaper}/opt/REAPER"
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
            ${extensionSetup}

            echo "[fts] Starting Xvfb on $DISPLAY..."
            ${pkgs.xorg-server}/bin/Xvfb "$DISPLAY" -screen 0 ${cfg.headless.resolution} -nolisten tcp &
            XVFB_PID=$!
            for i in $(seq 1 20); do
              if ${pkgs.xset}/bin/xset q &>/dev/null 2>&1; then break; fi
              sleep 0.1
            done

            cleanup() {
              echo "[fts] Cleaning up..."
              kill "$XVFB_PID" 2>/dev/null || true
              pkill -f "reaper.*-newinst" 2>/dev/null || true
            }
            trap cleanup EXIT

            echo "[fts] Xvfb ready on $DISPLAY"
            export FTS_REAPER_EXECUTABLE="${reaper}/bin/reaper"

            if [ $# -gt 0 ]; then exec "$@";
            else echo "[fts] No command given — dropping into shell."; exec bash; fi
          '';

          fts-test = pkgs.writeShellScriptBin "fts-test" ''
            exec ${reaper-fhs}/bin/reaper-env ${reaper-headless}/bin/reaper-headless "$@"
          '';

          fts-gui = pkgs.writeShellScriptBin "fts-gui" ''
            ${extensionSetup}
            exec ${reaper-fhs}/bin/reaper-env ${reaper}/bin/reaper "$@"
          '';
        in
        {
          inherit
            reaper-fhs
            reaper-headless
            fts-test
            fts-gui
            reaper
            sws
            reapack
            ;
        };

      # ── Preset configs ─────────────────────────────────────────

      defaultConfig = {
        extensions = {
          sws = true;
          reapack = true;
        };
        plugins = {
          lv2 = false;
          vst = false;
          vst3 = false;
          clap = false;
          ladspa = false;
        };
        audio = {
          pipewire = true;
          pulseaudio = true;
          alsa = true;
          jack = true;
        };
        codecs = {
          ffmpeg = false;
          lame = true;
          vorbis = true;
          ogg = true;
          flac = true;
          opus = true;
          sndfile = true;
        };
        headless = {
          enable = true;
          resolution = "1920x1080x24";
          display = ":99";
        };
      };

      presets = {
        ci = defaultConfig // {
          extensions = {
            sws = false;
            reapack = false;
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
          headless = {
            enable = true;
            resolution = "1280x720x16";
            display = ":99";
          };
        };

        dev = defaultConfig // {
          plugins = {
            lv2 = true;
            vst = false;
            vst3 = false;
            clap = true;
            ladspa = false;
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
        };

        full = defaultConfig // {
          plugins = {
            lv2 = true;
            vst = false;
            vst3 = false;
            clap = true;
            ladspa = true;
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
          headless = {
            enable = false;
            resolution = "1920x1080x24";
            display = ":99";
          };
        };
      };
    in
    {
      # Expose for consumers
      inherit presets;
      lib.mkFtsPackages = mkFtsPackages;
    }
    // flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          config.allowUnfreePredicate =
            pkg:
            builtins.elem (pkgs.lib.getName pkg) [
              "reaper"
            ];
        };

        ciPkgs = mkFtsPackages {
          inherit pkgs;
          cfg = presets.ci;
        };
        devPkgs = mkFtsPackages {
          inherit pkgs;
          cfg = presets.dev;
        };
        defaultPkgs = mkFtsPackages {
          inherit pkgs;
          cfg = defaultConfig;
        };
        fullPkgs = mkFtsPackages {
          inherit pkgs;
          cfg = presets.full;
        };
      in
      {
        # ── Packages (unchanged from before) ────────────────────
        packages = {
          default = defaultPkgs.fts-test;
          fts-test = defaultPkgs.fts-test;
          fts-test-ci = ciPkgs.fts-test;
          fts-test-dev = devPkgs.fts-test;
          fts-gui = defaultPkgs.fts-gui;
          fts-gui-dev = devPkgs.fts-gui;
          reaper-fhs = defaultPkgs.reaper-fhs;
        };

        # ── devenv-powered dev shells ───────────────────────────
        devShells = {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, config, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    devPkgs.fts-test
                    devPkgs.fts-gui
                    devPkgs.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    FTS_REAPER_EXECUTABLE = "${devPkgs.reaper}/bin/reaper";
                    FTS_REAPER_RESOURCES = "${devPkgs.reaper}/opt/REAPER";
                  };

                  scripts = {
                    fts-smoke.exec = ''
                      echo "[fts] Smoke test: starting REAPER..."
                      fts-test bash -c '"$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors & RPID=$!; sleep 3; kill -0 $RPID && echo "PASSED" && kill $RPID || (echo "FAILED"; exit 1)'
                    '';
                  };

                  enterShell = ''
                    echo ""
                    echo "  fts-flake dev shell (devenv)"
                    echo "  ────────────────────────────────────────"
                    echo "  fts-test [cmd]  — headless FHS env (CI-ready)"
                    echo "  fts-gui         — launch REAPER with GUI"
                    echo "  reaper-env      — drop into bare FHS shell"
                    echo "  fts-smoke       — quick REAPER smoke test"
                    echo ""
                    echo "  REAPER:  ${devPkgs.reaper}/bin/reaper"
                    echo "  SWS:     enabled"
                    echo "  ReaPack: enabled"
                    echo ""
                  '';

                  git-hooks.hooks = {
                    nixfmt.enable = true;
                  };
                }
              )
            ];
          };

          ci = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    ciPkgs.fts-test
                    ciPkgs.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    FTS_REAPER_EXECUTABLE = "${ciPkgs.reaper}/bin/reaper";
                    FTS_REAPER_RESOURCES = "${ciPkgs.reaper}/opt/REAPER";
                  };
                }
              )
            ];
          };

          minimal = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    defaultPkgs.fts-test
                    defaultPkgs.fts-gui
                    defaultPkgs.reaper-fhs
                  ];

                  env = {
                    FTS_REAPER_EXECUTABLE = "${defaultPkgs.reaper}/bin/reaper";
                    FTS_REAPER_RESOURCES = "${defaultPkgs.reaper}/opt/REAPER";
                  };
                }
              )
            ];
          };
        };

        checks.reaper-starts = pkgs.runCommand "reaper-starts" { } ''
          test -x ${pkgs.reaper}/opt/REAPER/reaper
          echo "REAPER binary OK" > $out
        '';
      }
    );
}
