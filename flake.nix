{
  description = "Reusable Nix flake for Dioxus applications — web, desktop, and mobile";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, flake-parts, crane, ... } @inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "x86_64-darwin" "aarch64-darwin" "aarch64-linux" ];

      perSystem = { self', pkgs, lib, system, ... }:
        let
          # ── Rust toolchain ───────────────────────────────────────────
          rustToolchain = pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
            targets = [ "wasm32-unknown-unknown" ];
          };

          # ── Crane ────────────────────────────────────────────────────
          craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

          rev = toString (self.shortRev or self.dirtyShortRev or self.lastModified or "unknown");

          # ── Source filtering ─────────────────────────────────────────
          # Include Rust sources plus Dioxus asset files
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              (craneLib.fileset.commonCargoSources ./.)
              (lib.fileset.fileFilter (f: f.hasExt "css") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "html") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "ico") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "png") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "jpg") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "svg") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "ttf") ./.)
              (lib.fileset.fileFilter (f: f.hasExt "woff2") ./.)
              (lib.fileset.fileFilter (f: f.name == "Dioxus.toml") ./.)
              (lib.fileset.fileFilter (f: f.name == "tailwind.config.js") ./.)
            ];
          };

          # ── Build dependencies (platform-specific) ──────────────────
          buildInputs = (with pkgs; [
            openssl openssl.dev pkg-config fontconfig freetype
          ])
          ++ lib.optionals pkgs.stdenv.isLinux (with pkgs; [
            # WebView / GTK (Dioxus desktop)
            glib gtk3 libsoup_3 webkitgtk_4_1 xdotool
            # X11 / Wayland
            libx11 libxcursor libxrandr libxi libxcb
            libxkbcommon wayland
            # Graphics
            libGL vulkan-loader
            # Multimedia (webkit2gtk media playback)
            gst_all_1.gstreamer gst_all_1.gst-plugins-base
            gst_all_1.gst-plugins-good gst_all_1.gst-plugins-bad
          ])
          ++ lib.optionals pkgs.stdenv.isDarwin (with pkgs; [
            apple-sdk_15
            libiconv
          ]);

          # Stub codesign for Nix sandbox — dx tries to invoke codesign on macOS
          fakeCodesign = pkgs.writeShellScriptBin "codesign" ''exec true'';

          nativeBuildInputs = with pkgs; [
            pkg-config
            rustPlatform.bindgenHook
            dioxus-cli
            wasm-bindgen-cli
            tailwindcss_4
          ] ++ lib.optionals pkgs.stdenv.isDarwin [
            fakeCodesign
          ];

          # ── Common crane args ────────────────────────────────────────
          commonArgs = {
            inherit src buildInputs nativeBuildInputs;
            pname = "dioxus-workspace";
            version = rev;
            strictDeps = true;
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            CC_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.clang}/bin/clang";
            AR_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.bintools}/bin/llvm-ar";
          };

          # ── Cached workspace deps ────────────────────────────────────
          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          # ── Runtime library path ─────────────────────────────────────
          libPath = lib.makeLibraryPath (with pkgs;
            [ fontconfig freetype openssl ]
            ++ lib.optionals pkgs.stdenv.isLinux [
              libGL vulkan-loader gtk3 glib
              libx11 libxcb libxkbcommon wayland
              webkitgtk_4_1 libsoup_3
              gst_all_1.gstreamer gst_all_1.gst-plugins-base
              gst_all_1.gst-plugins-good gst_all_1.gst-plugins-bad
            ]
          );

        in {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          formatter = pkgs.nixfmt-rfc-style;

          # ============================================================
          # Packages
          # ============================================================
          packages = {
            deps = cargoArtifacts;

            # Web app (WASM via dx)
            web = craneLib.buildPackage (commonArgs // {
              pname = "web";
              version = rev;
              inherit cargoArtifacts;
              doNotPostBuildInstallCargoBinaries = true;
              buildPhaseCargoCommand = ''
                cd packages/web
                dx build --release --platform web
              '';
              installPhaseCommand = ''
                cd "$NIX_BUILD_TOP/source"
                mkdir -p $out/www
                cp -r target/dx/web/release/web/* $out/www/
              '';
              doCheck = false;
            });

            # Desktop app (native webview via dx)
            desktop = craneLib.buildPackage (commonArgs // {
              pname = "desktop";
              version = rev;
              inherit cargoArtifacts;
              doNotPostBuildInstallCargoBinaries = true;
              buildPhaseCargoCommand = ''
                cd packages/desktop
                dx build --release --platform desktop
              '';
              installPhaseCommand = ''
                cd "$NIX_BUILD_TOP/source"
                mkdir -p $out/bin
                if [ -d "target/dx/desktop/release/macos" ]; then
                  mkdir -p $out/Applications
                  cp -r target/dx/desktop/release/macos/*.app $out/Applications/
                  ln -s "$out/Applications/"*.app"/Contents/MacOS/"* $out/bin/desktop
                elif [ -f "target/release/desktop" ]; then
                  cp target/release/desktop $out/bin/
                fi
              '';
              doCheck = false;
            });

            default = self'.packages.desktop;
          };

          # ============================================================
          # Checks
          # ============================================================
          checks = {
            clippy = craneLib.cargoClippy (commonArgs // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- --deny warnings";
            });

            fmt = craneLib.cargoFmt {
              inherit src;
              pname = "dioxus-workspace";
              version = rev;
            };

            tests = craneLib.cargoNextest (commonArgs // {
              inherit cargoArtifacts;
              partitions = 1;
              partitionType = "count";
            });
          };

          # ============================================================
          # Dev Shell
          # ============================================================
          devShells.default = pkgs.mkShell {
            inputsFrom = [ (craneLib.buildDepsOnly commonArgs) ];

            packages = with pkgs; [
              rustToolchain
              dioxus-cli
              wasm-bindgen-cli
              tailwindcss_4
              cargo-watch
              cargo-nextest
              bacon
              nodejs_22
            ]
            ++ buildInputs
            ++ nativeBuildInputs;

            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
            OPENSSL_DIR = "${pkgs.openssl.dev}";
            OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
            CC_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.clang}/bin/clang";
            AR_wasm32_unknown_unknown = "${pkgs.llvmPackages_18.bintools}/bin/llvm-ar";
            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            LD_LIBRARY_PATH = lib.optionalString pkgs.stdenv.isLinux libPath;
            XDG_DATA_DIRS = lib.optionalString pkgs.stdenv.isLinux
              "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}";

            shellHook = ''
              echo ""
              echo "  Dioxus dev shell"
              echo "  Rust: $(rustc --version)"
              echo "  dx:   $(dx --version 2>/dev/null || echo 'not available')"
              echo ""
            '';
          };
        };
    };
}
