{
  description = "Task — spec-driven task management";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    crane.url = "github:ipetkov/crane";

    dioxus-flake = {
      url = "github:FastTrackStudios/Dioxus-Flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
      inputs.crane.follows = "crane";
    };
  };

  outputs = inputs@{ flake-parts, nixpkgs, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      # ── NixOS module (for deploying task-server) ──────────────────
      flake = {
        nixosModules.default = ./nix/module.nix;
        nixosModules.task-server = ./nix/module.nix;
      };

      perSystem = { system, lib, self', ... }:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          # ── Rust toolchain ──────────────────────────────────────────────
          rustToolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustToolchain;

          # ── Source layout ───────────────────────────────────────────────
          voxSrc = builtins.path {
            path = /home/cody/Development/FastTrackStudio/vox;
            name = "vox-src";
          };

          ftsUiSrc = builtins.path {
            path = /home/cody/Development/FastTrackStudio/Plugins/fts-ui;
            name = "fts-ui-src";
          };

          injectDeps = ''
            mkdir -p "$NIX_BUILD_TOP/FastTrackStudio/vox"
            cp -rL ${voxSrc}/. "$NIX_BUILD_TOP/FastTrackStudio/vox/"
            chmod -R u+w "$NIX_BUILD_TOP/FastTrackStudio/vox"

            mkdir -p "$NIX_BUILD_TOP/FastTrackStudio/Plugins/fts-ui"
            cp -rL ${ftsUiSrc}/. "$NIX_BUILD_TOP/FastTrackStudio/Plugins/fts-ui/"
            chmod -R u+w "$NIX_BUILD_TOP/FastTrackStudio/Plugins/fts-ui"
          '';

          taskSrc = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let name = builtins.baseNameOf (toString path); in
              !(builtins.elem name [ "target" "node_modules" ".git" ".beads" "result" ]);
          };

          cargoVendorDir = craneLib.vendorCargoDeps { src = taskSrc; };

          commonArgs = {
            src = taskSrc;
            inherit cargoVendorDir;
            strictDeps = true;
            postPatch = injectDeps;
          };

          # Server-only args (no GUI deps needed)
          serverArgs = commonArgs // {
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [ openssl ];
          };

          # ── wasm-bindgen-cli pinned to match Cargo.lock ────────────────
          wasm-bindgen-cli = pkgs.rustPlatform.buildRustPackage rec {
            pname = "wasm-bindgen-cli";
            version = "0.2.116";
            src = pkgs.fetchCrate {
              inherit pname version;
              hash = "sha256-Pcb5+dEaWK+SQJz73/3Si0kxEy8y8n21t+DETMdvKHc=";
            };
            cargoHash = "sha256-PztHuoWBh+wqOuSFTQxnnddKSSu0S1MBOgKy0szHQpI=";
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = lib.optionals pkgs.stdenv.isLinux [ pkgs.openssl ]
              ++ lib.optionals pkgs.stdenv.isDarwin
                (with pkgs.darwin.apple_sdk.frameworks; [ Security CoreFoundation ]);
            doCheck = false;
          };

          # ── Packages ────────────────────────────────────────────────────

          task-server = craneLib.buildPackage (serverArgs // {
            pname = "task-server";
            cargoExtraArgs = "--package task-server";
            doCheck = false;
          });

          task-cli = craneLib.buildPackage (serverArgs // {
            pname = "task-cli";
            cargoExtraArgs = "--package task-cli";
            doCheck = false;
          });

          xtask = craneLib.buildPackage (commonArgs // {
            pname = "xtask";
            cargoExtraArgs = "--package xtask";
            doCheck = false;
          });

          vault-core = craneLib.buildPackage (commonArgs // {
            pname = "vault-core";
            cargoExtraArgs = "--package vault-core";
          });

          cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
            pname = "task-deps";
            version = "0.1.0";
          });

          obsidian-wasm = craneLib.mkCargoDerivation (commonArgs // {
            pname = "obsidian-plugin-wasm";
            CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
            buildPhaseCargoCommand = ''
              cargo build --release \
                --package obsidian-plugin \
                --target wasm32-unknown-unknown
            '';
            installPhase = ''
              mkdir -p $out/lib
              cp target/wasm32-unknown-unknown/release/obsidian_plugin.wasm $out/lib/
            '';
            doCheck = false;
          });

        in
        {
          packages = {
            default = task-cli;
            inherit task-server task-cli xtask vault-core obsidian-wasm wasm-bindgen-cli;
          } // lib.optionalAttrs pkgs.stdenv.isLinux {
            task-desktop = craneLib.buildPackage (commonArgs // {
              pname = "task-desktop";
              cargoExtraArgs = "--package task-desktop";
              nativeBuildInputs = with pkgs; [
                pkg-config
                gobject-introspection
                wrapGAppsHook4
              ];
              buildInputs = with pkgs; [
                gtk4
                webkitgtk_4_1
                libsoup_3
                openssl
              ];
              doCheck = false;
            });
          };

          # ── Checks ──────────────────────────────────────────────────────
          checks = {
            vault-core-tests = craneLib.cargoTest (commonArgs // {
              inherit cargoArtifacts;
              pname = "vault-core-tests";
              version = "0.1.0";
              cargoExtraArgs = "--package vault-core";
            });
            fmt = craneLib.cargoFmt { src = taskSrc; pname = "task-fmt"; version = "0.1.0"; };
          };

          # ── Dev shell ───────────────────────────────────────────────────
          devShells.default = pkgs.mkShell {
            inputsFrom = [ inputs.dioxus-flake.devShells.${system}.default ];

            nativeBuildInputs = with pkgs; [
              rustToolchain
              cargo-watch
              cargo-expand
              wasm-pack
              wasm-bindgen-cli
              binaryen
              nodejs_22
              nodePackages.npm
              pkg-config
            ] ++ lib.optionals pkgs.stdenv.isLinux (with pkgs; [
              gtk4.dev
              gobject-introspection
              openssl.dev
            ]);

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            shellHook = ''
              echo ""
              echo "  task dev shell"
              echo "  ──────────────────────────────────────────────"
              echo "  nix build .#task-server   HTTP API server"
              echo "  nix build .#task-cli      CLI tool"
              echo "  nix build .#task-desktop   Desktop app (Linux)"
              echo "  nix build .#obsidian-wasm  Obsidian plugin WASM"
              echo "  nix flake check           tests, clippy, fmt"
              echo ""
            '';
          };
        };
    };
}
