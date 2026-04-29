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

    vox = {
      flake = false;
      url = "github:Codys-Wright/vox";
    };

    ftsUi = {
      flake = false;
      url = "github:FastTrackStudios/fts-ui";
    };

    betterAuth = {
      flake = false;
      url = "github:better-auth-rs/better-auth-rs";
    };

    seaOrm = {
      flake = false;
      url = "github:SeaQL/sea-orm";
    };

    crudcrate = {
      flake = false;
      url = "github:evanjt/crudcrate";
    };

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
        nixosModules.task-email-watcher = ./nix/task-email-watcher.nix;
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
          taskSrc = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let name = builtins.baseNameOf (toString path); in
              !(builtins.elem name [ "target" "node_modules" ".git" ".beads" "result" ]);
          };

          taskCliSrc = pkgs.runCommand "task-cli-src" { } ''
            set -euo pipefail

            mkdir -p "$out/crates"
            cp -r ${taskSrc}/Cargo.lock "$out/Cargo.lock"
            cp -r ${taskSrc}/rust-toolchain.toml "$out/rust-toolchain.toml"
            if [ -e ${taskSrc}/.cargo ]; then
              cp -r ${taskSrc}/.cargo "$out/.cargo"
            fi
            cp -r ${taskSrc}/crates/task-core "$out/crates/task-core"
            cp -r ${taskSrc}/crates/task-cli "$out/crates/task-cli"

            cat > "$out/Cargo.toml" <<'EOF'
[workspace]
members = [
    "crates/task-core",
    "crates/task-cli",
]
resolver = "2"

[workspace.metadata.crane]
name = "task"

[workspace.package]
version = "0.1.0"
authors = ["Cody Wright"]
edition = "2021"

[workspace.dependencies]
chrono = "0.4"
eyre = "0.6"
rrule = "0.11"
thiserror = "2.0"
tracing = "0.1"

[workspace.dependencies.facet]
version = "0.44.1"
features = [
    "reflect",
    "chrono",
    "uuid",
]

[workspace.dependencies.facet-core]
version = "0.44.1"

[workspace.dependencies.facet-json]
version = "0.44.1"

[workspace.dependencies.facet-pretty]
version = "0.44.1"

[workspace.dependencies.facet-reflect]
version = "0.44.1"

[workspace.dependencies.facet-yaml]
version = "0.44.1"

[workspace.dependencies.moire]
git = "https://github.com/bearcove/moire"
rev = "13eac79"

[workspace.dependencies.moire-types]
git = "https://github.com/bearcove/moire"
rev = "13eac79"

[workspace.dependencies.vox]
git = "https://github.com/Codys-Wright/vox"
rev = "4103658f29c88ffd913314903f16e327c5248cdb"

[workspace.dependencies.vox-core]
git = "https://github.com/Codys-Wright/vox"
rev = "4103658f29c88ffd913314903f16e327c5248cdb"

[workspace.dependencies.serde]
version = "1"
features = ["derive"]

[workspace.dependencies.serde_json]
version = "1"

[workspace.dependencies.tokio]
version = "1"
features = ["full"]

[workspace.dependencies.uuid]
version = "1"
features = [
    "v4",
    "v7",
]
EOF

            chmod -R u+w "$out"
          '';

          cargoVendorDir = craneLib.vendorCargoDeps { src = taskSrc; };
          cargoVendorDirCli = craneLib.vendorCargoDeps { src = taskCliSrc; };

          commonArgs = {
            src = taskSrc;
            inherit cargoVendorDir;
            strictDeps = true;
          };

          # Server-only args (no GUI deps needed)
          serverArgs = commonArgs // {
            nativeBuildInputs = with pkgs; [ pkg-config ];
            buildInputs = with pkgs; [ openssl ];
          };

          cliArgs = commonArgs // {
            src = taskCliSrc;
            cargoVendorDir = cargoVendorDirCli;
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

          task-cli = craneLib.buildPackage (cliArgs // {
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
