{
  description = "A minimal flake for building the Rust binary 'alors'";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "alors";
          version = "0.1.0";
          src = pkgs.lib.cleanSource ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "openrouter_api-0.1.6" = "sha256-0hEZFTEymddJiyReZ3hOme8cBZy8zkbvrJekHT1g2w4=";
            };
          };
        };
      }
    );
}
