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

    # Forks consumed only by the Nix build. `cargo build` uses the live
    # sibling checkouts in ../FastTrackStudio/{sea-orm,crudcrate,better-auth};
    # bump these revs when you want the packaged build to track new fork work.
    sea-orm-fork = {
      flake = false;
      url = "github:FastTrackStudios/sea-orm";
    };
    crudcrate-fork = {
      flake = false;
      url = "github:FastTrackStudios/crudcrate";
    };
    better-auth-fork = {
      flake = false;
      url = "github:FastTrackStudios/better-auth";
    };

    # Shared Dioxus toolchain (web/desktop/mobile/native) — re-exposed
    # below as `devShells.ui` and `devShells.mobile`.
    dioxus-flake = {
      url = "path:/home/cody/Development/Dioxus/dioxus-flake";
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

          taskPackageSrc = pkgs.runCommand "task-package-src" { } ''
            set -euo pipefail

            cp -r ${taskSrc} "$out"
            chmod -R u+w "$out"

            mkdir -p "$out/nix-external/FastTrackStudio"
            cp -r ${inputs.sea-orm-fork} "$out/nix-external/FastTrackStudio/sea-orm"
            cp -r ${inputs.crudcrate-fork} "$out/nix-external/FastTrackStudio/crudcrate"
            cp -r ${inputs.better-auth-fork} "$out/nix-external/FastTrackStudio/better-auth"
            cp -r ${inputs.vox} "$out/nix-external/FastTrackStudio/vox"
            chmod -R u+w "$out/nix-external"

            substituteInPlace "$out/Cargo.toml" \
              --replace-fail 'path = "../FastTrackStudio/sea-orm"' 'path = "nix-external/FastTrackStudio/sea-orm"' \
              --replace-fail 'path = "../FastTrackStudio/sea-orm/sea-orm-migration"' 'path = "nix-external/FastTrackStudio/sea-orm/sea-orm-migration"' \
              --replace-fail 'path = "../FastTrackStudio/crudcrate/crudcrate"' 'path = "nix-external/FastTrackStudio/crudcrate/crudcrate"' \
              --replace-fail 'path = "../FastTrackStudio/better-auth/crates/core"' 'path = "nix-external/FastTrackStudio/better-auth/crates/core"' \
              --replace-fail 'path = "../FastTrackStudio/better-auth/crates/api"' 'path = "nix-external/FastTrackStudio/better-auth/crates/api"' \
              --replace-fail 'path = "../FastTrackStudio/better-auth/crates/derive"' 'path = "nix-external/FastTrackStudio/better-auth/crates/derive"'

            substituteInPlace "$out/apps/server/Cargo.toml" \
              --replace-fail 'path = "../../../FastTrackStudio/better-auth"' 'path = "../../nix-external/FastTrackStudio/better-auth"' \
              --replace-fail 'path = "../../../FastTrackStudio/better-auth/crates/core"' 'path = "../../nix-external/FastTrackStudio/better-auth/crates/core"' \
              --replace-fail 'path = "../../../FastTrackStudio/better-auth/crates/api"' 'path = "../../nix-external/FastTrackStudio/better-auth/crates/api"'

            if [ -f "$out/nix-external/FastTrackStudio/better-auth/crates/api/Cargo.toml" ]; then
              ${pkgs.perl}/bin/perl -0pi -e 's#vox-types = \{ path = "../../../vox/rust/vox-types", optional = true \}#vox-types = { git = "https://github.com/Codys-Wright/vox", rev = "ccb46b5416100d38430cb082d71bf0377428c241", optional = true }#g' \
                "$out/nix-external/FastTrackStudio/better-auth/crates/api/Cargo.toml"
            fi

            substituteInPlace "$out/crates/task-db/Cargo.toml" \
              --replace-fail 'path = "../../../FastTrackStudio/better-auth/crates/core"' 'path = "../../nix-external/FastTrackStudio/better-auth/crates/core"'

            substituteInPlace "$out/nix-external/FastTrackStudio/crudcrate/crudcrate/Cargo.toml" \
              --replace-fail 'spring-rs = ["spring", "spring-web", "derive"]' 'spring-rs = ["derive"]' \
              --replace-fail 'spring = { version = "0.4.3", optional = true }' '# spring removed for packaged build' \
              --replace-fail 'spring-web = { version = "0.4.3", optional = true }' '# spring-web removed for packaged build'
            ${pkgs.perl}/bin/perl -0pi -e 's/\n\[dev-dependencies\].*?(?=\n\[\[example\]\])/\n/s' \
              "$out/nix-external/FastTrackStudio/crudcrate/crudcrate/Cargo.toml"
            ${pkgs.perl}/bin/perl -0pi -e 's/\n\[dev-dependencies\].*?(?=\n\[lints\.clippy\])/\n/s' \
              "$out/nix-external/FastTrackStudio/crudcrate/crudcrate-derive/Cargo.toml"
          '';

          taskCliSrc = pkgs.runCommand "task-cli-src" { } ''
            set -euo pipefail

            mkdir -p "$out/crates"
            cp -r ${taskPackageSrc}/Cargo.lock "$out/Cargo.lock"
            cp -r ${taskPackageSrc}/rust-toolchain.toml "$out/rust-toolchain.toml"
            if [ -e ${taskPackageSrc}/.cargo ]; then
              cp -r ${taskPackageSrc}/.cargo "$out/.cargo"
            fi
            cp -r ${taskPackageSrc}/crates/task-core "$out/crates/task-core"
            cp -r ${taskPackageSrc}/crates/task-cli "$out/crates/task-cli"

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
rev = "ccb46b5416100d38430cb082d71bf0377428c241"

[workspace.dependencies.vox-core]
git = "https://github.com/Codys-Wright/vox"
rev = "ccb46b5416100d38430cb082d71bf0377428c241"

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

          cargoVendorDir = craneLib.vendorCargoDeps { src = taskPackageSrc; };
          cargoVendorDirCli = craneLib.vendorCargoDeps { src = taskCliSrc; };

          commonArgs = {
            src = taskPackageSrc;
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
            version = "0.1.0";
            cargoExtraArgs = "--package task-server";
            doCheck = false;
          });

          task-cli = craneLib.buildPackage (cliArgs // {
            pname = "task-cli";
            version = "0.1.0";
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
            inherit cargoArtifacts;
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
              openssl.dev
            ];

            RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";

            shellHook = ''
              echo ""
              echo "  task dev shell"
              echo "  ──────────────────────────────────────────────"
              echo "  nix build .#task-server   Vox/CRDT service"
              echo "  nix build .#task-cli      CLI tool"
              echo "  nix build .#obsidian-wasm  Obsidian plugin WASM"
              echo "  nix flake check           tests, clippy, fmt"
              echo "  nix develop .#ui          Dioxus web/desktop dev shell"
              echo "  nix develop .#mobile      Dioxus mobile (Android) dev shell"
              echo ""
            '';
          };

          # Re-export the shared Dioxus dev shells so app work uses the
          # same toolchain as fts-ui / dioxus-flake without duplicating
          # config here.
          devShells.ui = inputs.dioxus-flake.devShells.${system}.default;
          devShells.mobile = inputs.dioxus-flake.devShells.${system}.mobile;
        };
    };
}
