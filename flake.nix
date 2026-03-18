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
            ++ pkgs.lib.optionals cfg.audio.pipewire [ pipewire wireplumber ]
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
              REAPER_CONFIG="${cfg.reaper.configDir}"
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
              export FTS_REAPER_CONFIG="${cfg.reaper.configDir}"
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

            REAPER_HOME="${cfg.reaper.configDir}"
            mkdir -p "$REAPER_HOME"
            ${extensionSetup}

            echo "[fts] Starting Xvfb on $DISPLAY..."
            ${pkgs.xorg-server}/bin/Xvfb "$DISPLAY" -screen 0 ${cfg.headless.resolution} -nolisten tcp &
            XVFB_PID=$!
            for i in $(seq 1 20); do
              if ${pkgs.xset}/bin/xset q &>/dev/null 2>&1; then break; fi
              sleep 0.1
            done

            # Start PipeWire audio stack if available (provides JACK-compatible audio)
            PIPEWIRE_PID=""
            WIREPLUMBER_PID=""
            if command -v pipewire &>/dev/null; then
              echo "[fts] Starting PipeWire audio stack..."

              # XDG_RUNTIME_DIR must exist and be writable — create inside /run/user
              # or /tmp since we're in an FHS sandbox. PipeWire creates its sockets here.
              export XDG_RUNTIME_DIR="/run/user/$(id -u)"
              mkdir -p "$XDG_RUNTIME_DIR" 2>/dev/null || {
                export XDG_RUNTIME_DIR="/tmp/fts-runtime-$$"
                mkdir -p "$XDG_RUNTIME_DIR"
              }
              chmod 700 "$XDG_RUNTIME_DIR"

              # Use a drop-in config to disable protocol-pulse (avoids
              # "Address already in use" when pipewire-pulse is also loaded).
              # Also disable dbus since we're headless.
              export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
              mkdir -p "$XDG_CONFIG_HOME/pipewire/pipewire.conf.d"
              cat > "$XDG_CONFIG_HOME/pipewire/pipewire.conf.d/99-headless.conf" << 'PWEOF'
            # Headless CI overrides
            context.properties = {
              support.dbus = false
            }
            PWEOF

              # 1. PipeWire core daemon (uses default config + our drop-in)
              pipewire &
              PIPEWIRE_PID=$!
              sleep 0.5

              # 2. WirePlumber session manager (sets up JACK graph)
              if command -v wireplumber &>/dev/null; then
                wireplumber &
                WIREPLUMBER_PID=$!
                sleep 0.5
              fi

              # Verify PipeWire is running
              if kill -0 "$PIPEWIRE_PID" 2>/dev/null; then
                echo "[fts] PipeWire audio stack ready (pipewire=$PIPEWIRE_PID wireplumber=$WIREPLUMBER_PID)"
                echo "[fts] XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"

                # Export FTS_REAPER_LAUNCHER so callers can prefix REAPER with pw-jack.
                # pw-jack sets LD_LIBRARY_PATH so JACK clients connect to PipeWire.
                if command -v pw-jack &>/dev/null; then
                  export FTS_REAPER_LAUNCHER="pw-jack"
                  echo "[fts] pw-jack available — set FTS_REAPER_LAUNCHER=pw-jack"
                fi
              else
                echo "[fts] WARNING: PipeWire failed to start"
              fi
            fi

            cleanup() {
              echo "[fts] Cleaning up..."
              [ -n "$WIREPLUMBER_PID" ] && kill "$WIREPLUMBER_PID" 2>/dev/null || true
              [ -n "$PIPEWIRE_PID" ] && kill "$PIPEWIRE_PID" 2>/dev/null || true
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
        # Canonical REAPER config directory. All rigs share this path.
        # Extensions, UserPlugins, Scripts, and reaper.ini live here.
        # Never touches ~/.config/REAPER/.
        reaper.configDir = "$HOME/.config/REAPER";
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
            pipewire = true;
            pulseaudio = false;
            alsa = true;
            jack = true;
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
                    FTS_REAPER_CONFIG = presets.dev.reaper.configDir;
                  };

                  # ── Tasks ───────────────────────────────────────
                  tasks = {
                    # Link SWS + ReaPack extensions into REAPER config
                    "reaper:setup-extensions" = {
                      exec = ''
                        REAPER_CONFIG="${presets.dev.reaper.configDir}"
                        mkdir -p "$REAPER_CONFIG/UserPlugins" "$REAPER_CONFIG/Scripts"
                        ln -sf "${devPkgs.sws}/UserPlugins/reaper_sws-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                        ln -sf "${devPkgs.sws}/Scripts/sws_python.py" "$REAPER_CONFIG/Scripts/"
                        ln -sf "${devPkgs.sws}/Scripts/sws_python64.py" "$REAPER_CONFIG/Scripts/"
                        ln -sf "${devPkgs.reapack}/UserPlugins/reaper_reapack-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                        echo "Extensions linked"
                      '';
                      status = ''
                        test -L "${presets.dev.reaper.configDir}/UserPlugins/reaper_sws-x86_64.so" && \
                        test -L "${presets.dev.reaper.configDir}/UserPlugins/reaper_reapack-x86_64.so"
                      '';
                      before = [ "devenv:enterShell" ];
                    };

                    # Smoke test: verify REAPER starts in headless FHS
                    "reaper:smoke" = {
                      exec = ''
                        fts-test bash -c '
                          "$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                          RPID=$!
                          sleep 3
                          if kill -0 $RPID 2>/dev/null; then
                            echo "REAPER running (PID $RPID)"
                            kill $RPID
                          else
                            echo "REAPER failed to start"
                            exit 1
                          fi
                        '
                      '';
                    };

                    # Build the daw workspace (when working on it)
                    "daw:build" = {
                      exec = "cargo build --workspace";
                      execIfModified = [
                        "Cargo.toml"
                        "Cargo.lock"
                        "crates/**/*.rs"
                        "apps/**/*.rs"
                      ];
                    };

                    # Run daw unit tests (no REAPER needed)
                    "daw:test" = {
                      exec = "cargo test --workspace";
                      after = [ "daw:build" ];
                    };

                    # Run REAPER integration tests (needs headless REAPER)
                    "daw:integration" = {
                      exec = ''
                        fts-test bash -c '
                          "$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                          RPID=$!
                          echo "Waiting for REAPER socket..."
                          for i in $(seq 1 30); do
                            SOCK=$(ls /tmp/fts-daw-*.sock 2>/dev/null | head -1)
                            if [ -n "$SOCK" ]; then break; fi
                            sleep 1
                          done
                          if [ -z "$SOCK" ]; then
                            echo "No socket found after 30s"
                            kill $RPID 2>/dev/null
                            exit 1
                          fi
                          echo "Socket ready: $SOCK"
                          cargo test -p daw-reaper -- --ignored --nocapture
                          STATUS=$?
                          kill $RPID 2>/dev/null
                          exit $STATUS
                        '
                      '';
                      after = [ "daw:build" ];
                    };

                    # Run all tests
                    "daw:ci" = {
                      exec = "echo 'All daw tests passed'";
                      after = [
                        "daw:test"
                        "daw:integration"
                      ];
                    };
                  };

                  enterShell = ''
                    echo ""
                    echo "  fts-flake dev shell (devenv)"
                    echo "  ────────────────────────────────────────"
                    echo "  fts-test [cmd]     — headless FHS env (CI-ready)"
                    echo "  fts-gui            — launch REAPER with GUI"
                    echo "  reaper-env         — drop into bare FHS shell"
                    echo "  fts-smoke          — REAPER headless smoke test"
                    echo "  fts-setup          — link extensions into REAPER config"
                    echo "  fts-integration    — run daw REAPER integration tests"
                    echo ""
                    echo "  REAPER:  ${devPkgs.reaper}/bin/reaper"
                    echo "  SWS:     enabled  |  ReaPack: enabled"
                    echo ""
                  '';

                  # ── Scripts ─────────────────────────────────────
                  scripts = {
                    fts-smoke.exec = ''
                      fts-test bash -c '
                        "$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                        RPID=$!
                        sleep 3
                        if kill -0 $RPID 2>/dev/null; then
                          echo "REAPER running (PID $RPID) — smoke test passed"
                          kill $RPID
                        else
                          echo "REAPER failed to start"
                          exit 1
                        fi
                      '
                    '';
                    fts-smoke.description = "Quick REAPER headless smoke test";

                    fts-integration.exec = ''
                      fts-test bash -c '
                        "$FTS_REAPER_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                        RPID=$!
                        echo "Waiting for REAPER socket..."
                        SOCK=""
                        for i in $(seq 1 30); do
                          SOCK=$(ls /tmp/fts-daw-*.sock 2>/dev/null | head -1)
                          if [ -n "$SOCK" ]; then break; fi
                          sleep 1
                        done
                        if [ -z "$SOCK" ]; then
                          echo "No socket found after 30s"
                          kill $RPID 2>/dev/null
                          exit 1
                        fi
                        echo "Socket ready: $SOCK"
                        cargo test -p daw-reaper -- --ignored --nocapture
                        STATUS=$?
                        kill $RPID 2>/dev/null
                        exit $STATUS
                      '
                    '';
                    fts-integration.description = "Run daw REAPER integration tests";

                    fts-setup.exec = ''
                      REAPER_CONFIG="${presets.dev.reaper.configDir}"
                      mkdir -p "$REAPER_CONFIG/UserPlugins" "$REAPER_CONFIG/Scripts"
                      ln -sf "${devPkgs.sws}/UserPlugins/reaper_sws-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                      ln -sf "${devPkgs.sws}/Scripts/sws_python.py" "$REAPER_CONFIG/Scripts/"
                      ln -sf "${devPkgs.sws}/Scripts/sws_python64.py" "$REAPER_CONFIG/Scripts/"
                      ln -sf "${devPkgs.reapack}/UserPlugins/reaper_reapack-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                      echo "Extensions linked into $REAPER_CONFIG"
                    '';
                    fts-setup.description = "Link SWS + ReaPack extensions into REAPER config";
                  };

                  # ── Claude Code integration ──────────────────────
                  claude.code = {
                    enable = true;
                    commands = {
                      smoke = ''
                        Run the REAPER headless smoke test

                        ```bash
                        fts-smoke
                        ```
                      '';
                      integration = ''
                        Run the full daw REAPER integration test suite

                        ```bash
                        fts-integration
                        ```
                      '';
                      build = ''
                        Build the daw workspace

                        ```bash
                        cargo build --workspace
                        ```
                      '';
                      test = ''
                        Run daw unit tests

                        ```bash
                        cargo test --workspace
                        ```
                      '';
                    };
                  };

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
                    FTS_REAPER_CONFIG = presets.ci.reaper.configDir;
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
                    FTS_REAPER_CONFIG = defaultConfig.reaper.configDir;
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
