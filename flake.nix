{
  description = "govee2mqtt dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    nixpkgs,
    fenix,
    ...
  }: let
    systems = [
      "x86_64-linux"
      "aarch64-linux"
    ];
    forAllSystems = f:
      nixpkgs.lib.genAttrs systems (
        system:
          f {
            inherit system;
            pkgs = nixpkgs.legacyPackages.${system};
          }
      );
  in {
    devShells = forAllSystems (
      {
        pkgs,
        system,
      }: let
        # Pinned stable toolchain via fenix. CI tracks
        # dtolnay/rust-toolchain@stable, so we follow stable here too. This
        # also sidesteps a stale ~/.rustup gcc-ld wrapper on the host that
        # points at a garbage-collected nix store path and breaks linking.
        rustToolchain = fenix.packages.${system}.stable.toolchain;
      in {
        default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.rust-analyzer

            # mosquitto-rs builds OpenSSL from source (vendored-openssl) via
            # cmake + perl; libsqlite3-sys (rusqlite bundled) compiles sqlite.
            pkgs.cmake
            pkgs.perl
            pkgs.pkg-config
          ];
        };
      }
    );

    formatter = forAllSystems ({pkgs, ...}: pkgs.alejandra);
  };
}
