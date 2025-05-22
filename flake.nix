{
  description = "Hanumail - An LSP server for email";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.11";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        hanumail = pkgs.rustPlatform.buildRustPackage {
          pname = "hanumail";
          version = "git";
          src = ./.;
          cargoLock = { lockFile = ./Cargo.lock; };
        };
      in with pkgs; {
        packages = {
          inherit hanumail;
          default = hanumail;
        };
        devShells.default = mkShell { buildInputs = [ gnumake ]; };
      });
}
