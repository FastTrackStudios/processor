{
  description = "architect — example-proto pattern (vox + Dioxus + SeaORM)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    # Dioxus dev shell — supplies rustc/cargo with wasm32 target,
    # wasm-bindgen-cli, binaryen, tailwindcss, dx, nodejs.
    dioxus-flake = {
      url = "github:FastTrackStudios/Dioxus-Flake";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      dioxus-flake,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        # Browser + driver for `wasm-bindgen-test-runner`. Pinned via
        # nixpkgs so a fresh checkout doesn't need a global install.
        wasmTestDeps = [
          pkgs.geckodriver
          pkgs.firefox
          pkgs.chromedriver
          pkgs.chromium
        ];

        # Everything else the workspace needs day-to-day. The Dioxus
        # shell already covers rustc/cargo/wasm-bindgen-cli; this list
        # is purely the architect-specific additions.
        extraDeps = with pkgs; [
          just
          sqlite # introspect the dev sqlite DB
          sea-orm-cli # migrations CLI
          wasm-pack # alternative wasm test runner
          pkg-config
          openssl
          curl
        ];

        # Base shell from dioxus-flake — gives us dx + cargo + wasm32 +
        # tailwindcss + wasm-bindgen-cli. We layer on the wasm test
        # harness deps and a shellHook that auto-wires the runner.
        baseShell = dioxus-flake.devShells.${system}.default;
      in
      {
        devShells = {
          default = pkgs.mkShell {
            inputsFrom = [ baseShell ];
            buildInputs = wasmTestDeps ++ extraDeps;

            shellHook = ''
              # Auto-wire `cargo test --target wasm32-unknown-unknown`:
              # cargo invokes wasm-bindgen-test-runner (from the dioxus
              # shell), which then drives geckodriver/firefox.
              export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner
              export GECKODRIVER=${pkgs.geckodriver}/bin/geckodriver
              export CHROMEDRIVER=${pkgs.chromedriver}/bin/chromedriver

              # Default DB path for `cargo run -p example-app-server` and
              # `cargo run -p example-app-db -- up`.
              : "''${DATABASE_URL:=sqlite://./example.db?mode=rwc}"
              export DATABASE_URL

              echo "architect dev shell"
              echo "  cargo, dx, wasm-bindgen-cli, geckodriver, firefox on PATH"
              echo "  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=$CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER"
              echo "  DATABASE_URL=$DATABASE_URL"
              echo
              echo "  just deps        — list workspace deps"
              echo "  just server      — run example-app-server"
              echo "  just test        — workspace cargo check + tests"
              echo "  just test-wasm   — browser integration tests"
            '';
          };

          # Compatibility alias — older docs/scripts still reference `.#ui`.
          ui = self.devShells.${system}.default;
        };
      }
    );
}
