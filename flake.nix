{
  description = "reaper-flake — reproducible, declarative REAPER DAW environment";

  inputs = {
    nixpkgs.url = "github:cachix/devenv-nixpkgs/rolling";
    devenv.url = "github:cachix/devenv";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";

    # reaper-file: typed Rust mappings for REAPER config / project / kbd files.
    # Used as the canonical reference for INI key names exposed by the
    # programs.reaper NixOS module in modules/reaper/default.nix.
    reaper-file = {
      url = "github:FastTrackStudios/reaper-file";
      flake = false;
    };
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
      reaper-file,
    } @ inputs:
    let
      # ── REAPER environment builder ─────────────────────────────────────────
      # Builds the REAPER FHS sandbox, headless runner, and scripts.
      # Used by both devenv modules and standalone package outputs.

      mkReaperPackages =
        { pkgs, cfg }:
        let
          reaper = pkgs.callPackage ./pkgs/reaper.nix {
            headless = cfg.headless.enable or false;
            jackLibrary = pkgs.pipewire.jack;
            libxml2 = pkgs.libxml2;
          };
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
            libepoxy
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
                echo "[reaper-flake] SWS extension linked"
              '';
              reapackSetup = pkgs.lib.optionalString cfg.extensions.reapack ''
                ln -sf "${reapack}/UserPlugins/reaper_reapack-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                echo "[reaper-flake] ReaPack extension linked"
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
              export REAPER_FLAKE_EXECUTABLE="${reaper}/bin/reaper"
              export REAPER_FLAKE_RESOURCES="${reaper}/opt/REAPER"
              export REAPER_FLAKE_CONFIG="${cfg.reaper.configDir}"
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

            REAPER_HOME="${cfg.reaper.configDir}"
            mkdir -p "$REAPER_HOME"
            ${extensionSetup}

            # Write a default reaper.ini if one doesn't exist.
            # REAPER on Linux uses [reaper] (lowercase) section headers.
            # audiodriver=1 selects JACK (provided by PipeWire below).
            if [ ! -f "$REAPER_HOME/reaper.ini" ]; then
              cat > "$REAPER_HOME/reaper.ini" << 'INI'
[reaper]
audiodriver=1
lastproject=
undomaxmem=0
[verchk]
audiocloseinactive=0
audioclosestop=0
INI
              echo "[reaper-flake] Default reaper.ini written to $REAPER_HOME"
            fi

            # Start a dedicated PipeWire instance so REAPER has a JACK backend.
            export XDG_RUNTIME_DIR="/tmp/reaper-flake-runtime-$$"
            export PIPEWIRE_RUNTIME_DIR="$XDG_RUNTIME_DIR"
            mkdir -p "$XDG_RUNTIME_DIR"
            pipewire &
            _REAPER_PW_PID=$!
            for i in $(seq 1 20); do
              [ -e "$XDG_RUNTIME_DIR/pipewire-0" ] && break
              sleep 0.1
            done
            if [ -e "$XDG_RUNTIME_DIR/pipewire-0" ]; then
              echo "[reaper-flake] PipeWire started (PID $_REAPER_PW_PID, runtime=$XDG_RUNTIME_DIR)"
              sleep 1
            else
              echo "[reaper-flake] WARNING: PipeWire socket not found after 2s"
            fi

            cleanup() {
              echo "[reaper-flake] Cleaning up..."
              pkill -f "reaper.*-newinst" 2>/dev/null || true
              [ -n "''${_REAPER_PW_PID:-}" ] && kill "$_REAPER_PW_PID" 2>/dev/null || true
            }
            trap cleanup EXIT

            echo "[reaper-flake] Headless mode ready (NOGDK libSwell — no X11 required)"
            export REAPER_FLAKE_EXECUTABLE="${reaper}/bin/reaper"
            export REAPER_FLAKE_RESOURCES="${reaper}/opt/REAPER"

            if [ $# -gt 0 ]; then
              exec "$@"
            else echo "[reaper-flake] No command given — dropping into shell."; exec bash; fi
          '';

          reaper-test = pkgs.writeShellScriptBin "reaper-test" ''
            exec ${reaper-fhs}/bin/reaper-env ${reaper-headless}/bin/reaper-headless "$@"
          '';

          reaper-gui = pkgs.writeShellScriptBin "reaper-gui" ''
            ${extensionSetup}
            exec ${reaper-fhs}/bin/reaper-env ${reaper}/bin/reaper "$@"
          '';

          reaper-native-gui = pkgs.writeShellScriptBin "reaper-native-gui" ''
            set -euo pipefail
            ${extensionSetup}

            # Run REAPER directly (no FHS sandbox) for native Wayland support
            export PATH="${reaper}/bin:$PATH"
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [
              pkgs.libGL pkgs.libepoxy pkgs.gtk3 pkgs.glib pkgs.cairo
              pkgs.pipewire pkgs.alsa-lib
            ]}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"

            exec ${reaper}/bin/reaper "$@"
          '';
        in
        {
          inherit
            reaper-fhs
            reaper-headless
            reaper-test
            reaper-gui
            reaper-native-gui
            reaper
            sws
            reapack
            ;
        };

      # ── Preset configs ──────────────────────────────────────────────────────

      defaultConfig = {
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
      lib.mkReaperPackages = mkReaperPackages;

      # ── NixOS / home-manager modules ─────────────────────────────────────
      # programs.reaper — dendritic option tree for reaper.ini configuration.
      # Keys mirror the typed structs in reaper-file/crates/reaper-config/.
      nixosModules.default = ./modules/reaper;
      nixosModules.reaper = ./modules/reaper;
    }
    # ── Cross-platform wrapper packages (Linux + macOS) ───────────────────
    // flake-utils.lib.eachSystem [
      "x86_64-linux"
      "aarch64-linux"
      "x86_64-darwin"
      "aarch64-darwin"
    ] (
      system:
      let
        wrapperPkgs = import nixpkgs {
          inherit system;
          config.allowUnfreePredicate =
            pkg:
            builtins.elem (nixpkgs.lib.getName pkg) [
              "reaper"
            ];
        };

        wrapperReaper = wrapperPkgs.callPackage ./wrapper/reaper/pkgs/reaper.nix {
          jackLibrary = wrapperPkgs.pipewire.jack or null;
        };
        wrapperSws = wrapperPkgs.callPackage ./wrapper/reaper/pkgs/sws.nix { };
        wrapperReapack = wrapperPkgs.callPackage ./wrapper/reaper/pkgs/reapack.nix { };
        wrapperIcon = nixpkgs.lib.optionalAttrs wrapperPkgs.stdenv.hostPlatform.isDarwin (
          wrapperPkgs.callPackage ./wrapper/reaper/pkgs/icon.nix { }
        );
        wrapperDmg = nixpkgs.lib.optionalAttrs wrapperPkgs.stdenv.hostPlatform.isDarwin (
          wrapperPkgs.callPackage ./wrapper/reaper/pkgs/dmg.nix {
            reaper = wrapperReaper;
            sws = wrapperSws;
            reapack = wrapperReapack;
            icon = wrapperIcon;
          }
        );
      in
      {
        wrapperPackages = {
          reaper = wrapperReaper;
          sws = wrapperSws;
          reapack = wrapperReapack;
        } // nixpkgs.lib.optionalAttrs wrapperPkgs.stdenv.hostPlatform.isDarwin {
          icon = wrapperIcon;
          dmg = wrapperDmg;
        };
      }
    )
    # ── Linux-only packages (FHS, headless, devenv) ───────────────────────
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

        ciPkgs = mkReaperPackages {
          inherit pkgs;
          cfg = presets.ci;
        };
        devPkgs = mkReaperPackages {
          inherit pkgs;
          cfg = presets.dev;
        };
        defaultPkgs = mkReaperPackages {
          inherit pkgs;
          cfg = defaultConfig;
        };
        fullPkgs = mkReaperPackages {
          inherit pkgs;
          cfg = presets.full;
        };
      in
      {
        packages = {
          default = defaultPkgs.reaper-test;
          reaper-test = defaultPkgs.reaper-test;
          reaper-test-ci = ciPkgs.reaper-test;
          reaper-test-dev = devPkgs.reaper-test;
          reaper-gui = defaultPkgs.reaper-gui;
          reaper-gui-dev = devPkgs.reaper-gui;
          reaper-native-gui = defaultPkgs.reaper-native-gui;
          reaper-fhs = defaultPkgs.reaper-fhs;
        };

        devShells = {
          default = devenv.lib.mkShell {
            inherit inputs pkgs;
            modules = [
              (
                { pkgs, config, ... }:
                {
                  cachix.pull = [ "fasttrackstudio" ];

                  packages = [
                    devPkgs.reaper-test
                    devPkgs.reaper-gui
                    devPkgs.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    REAPER_FLAKE_EXECUTABLE = "${devPkgs.reaper}/bin/reaper";
                    REAPER_FLAKE_RESOURCES = "${devPkgs.reaper}/opt/REAPER";
                    REAPER_FLAKE_CONFIG = presets.dev.reaper.configDir;
                  };

                  tasks = {
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

                    "reaper:smoke" = {
                      exec = ''
                        reaper-test bash -c '
                          "$REAPER_FLAKE_EXECUTABLE" -newinst -nosplash -ignoreerrors &
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

                    "daw:build" = {
                      exec = "cargo build --workspace";
                      execIfModified = [
                        "Cargo.toml"
                        "Cargo.lock"
                        "crates/**/*.rs"
                        "apps/**/*.rs"
                      ];
                    };

                    "daw:test" = {
                      exec = "cargo test --workspace";
                      after = [ "daw:build" ];
                    };

                    "daw:integration" = {
                      exec = ''
                        reaper-test bash -c '
                          "$REAPER_FLAKE_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                          RPID=$!
                          echo "Waiting for REAPER socket..."
                          for i in $(seq 1 30); do
                            SOCK=$(ls /tmp/reaper-flake-*.sock 2>/dev/null | head -1)
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
                    echo "  reaper-flake dev shell (devenv)"
                    echo "  ────────────────────────────────────────"
                    echo "  reaper-test [cmd]  — headless FHS env (CI-ready)"
                    echo "  reaper-gui         — launch REAPER with GUI"
                    echo "  reaper-env         — drop into bare FHS shell"
                    echo "  reaper-smoke       — REAPER headless smoke test"
                    echo "  reaper-setup       — link extensions into REAPER config"
                    echo "  reaper-integration — run daw REAPER integration tests"
                    echo ""
                    echo "  REAPER:  ${devPkgs.reaper}/bin/reaper"
                    echo "  SWS:     enabled  |  ReaPack: enabled"
                    echo ""
                  '';

                  scripts = {
                    reaper-smoke.exec = ''
                      reaper-test bash -c '
                        "$REAPER_FLAKE_EXECUTABLE" -newinst -nosplash -ignoreerrors &
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
                    reaper-smoke.description = "Quick REAPER headless smoke test";

                    reaper-integration.exec = ''
                      reaper-test bash -c '
                        "$REAPER_FLAKE_EXECUTABLE" -newinst -nosplash -ignoreerrors &
                        RPID=$!
                        echo "Waiting for REAPER socket..."
                        SOCK=""
                        for i in $(seq 1 30); do
                          SOCK=$(ls /tmp/reaper-flake-*.sock 2>/dev/null | head -1)
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
                    reaper-integration.description = "Run daw REAPER integration tests";

                    reaper-setup.exec = ''
                      REAPER_CONFIG="${presets.dev.reaper.configDir}"
                      mkdir -p "$REAPER_CONFIG/UserPlugins" "$REAPER_CONFIG/Scripts"
                      ln -sf "${devPkgs.sws}/UserPlugins/reaper_sws-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                      ln -sf "${devPkgs.sws}/Scripts/sws_python.py" "$REAPER_CONFIG/Scripts/"
                      ln -sf "${devPkgs.sws}/Scripts/sws_python64.py" "$REAPER_CONFIG/Scripts/"
                      ln -sf "${devPkgs.reapack}/UserPlugins/reaper_reapack-x86_64.so" "$REAPER_CONFIG/UserPlugins/"
                      echo "Extensions linked into $REAPER_CONFIG"
                    '';
                    reaper-setup.description = "Link SWS + ReaPack extensions into REAPER config";
                  };

                  claude.code = {
                    enable = true;
                    commands = {
                      smoke = ''
                        Run the REAPER headless smoke test

                        ```bash
                        reaper-smoke
                        ```
                      '';
                      integration = ''
                        Run the full daw REAPER integration test suite

                        ```bash
                        reaper-integration
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
                    ciPkgs.reaper-test
                    ciPkgs.reaper-fhs
                    pkgs.pkg-config
                    pkgs.openssl
                  ];

                  languages.rust = {
                    enable = true;
                    channel = "stable";
                  };

                  env = {
                    REAPER_FLAKE_EXECUTABLE = "${ciPkgs.reaper}/bin/reaper";
                    REAPER_FLAKE_RESOURCES = "${ciPkgs.reaper}/opt/REAPER";
                    REAPER_FLAKE_CONFIG = presets.ci.reaper.configDir;
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
                    defaultPkgs.reaper-test
                    defaultPkgs.reaper-gui
                    defaultPkgs.reaper-fhs
                  ];

                  env = {
                    REAPER_FLAKE_EXECUTABLE = "${defaultPkgs.reaper}/bin/reaper";
                    REAPER_FLAKE_RESOURCES = "${defaultPkgs.reaper}/opt/REAPER";
                    REAPER_FLAKE_CONFIG = defaultConfig.reaper.configDir;
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
