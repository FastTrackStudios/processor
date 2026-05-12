{
  description = "architect — example-proto pattern (vox + Dioxus + SeaORM)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    # Pulls in the Dioxus dev shell (dx + cargo + wasm32 target).
    # Re-exported as `devShells.${system}.{default, ui, mobile}`.
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
      in
      {
        # `nix develop` enters the Dioxus UI dev shell so dx + wasm32
        # + tailwindcss are on PATH alongside cargo.
        devShells.default = dioxus-flake.devShells.${system}.default;
        devShells.ui = dioxus-flake.devShells.${system}.default;
      }
    );
}
