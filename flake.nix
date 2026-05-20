{
  description = "Editor — Rust-native text editor for Dioxus, inspired by CodeMirror 6.";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    # Reuse the shared Dioxus flake's dev shell — it already bundles
    # the rust toolchain (from rust-toolchain.toml), `dx`, wasm-bindgen,
    # GTK/WebView/etc system deps for desktop, and the wasm32 target.
    # Pointed at the local path that Task uses; switch to
    # `github:FastTrackStudios/Dioxus-Flake` for a clean clone.
    dioxus-flake = {
      url = "path:/home/cody/Development/Dioxus/dioxus-flake";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-overlay.follows = "rust-overlay";
    };
  };

  outputs = inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];

      perSystem = { system, ... }: {
        # Inherit the Dioxus shared dev shell wholesale — it already
        # has every system lib `dx serve` wants. If we ever need
        # extras (a database CLI, mdbook, etc.) we can extend rather
        # than redefine.
        devShells.default = inputs.dioxus-flake.devShells.${system}.default;
      };
    };
}
