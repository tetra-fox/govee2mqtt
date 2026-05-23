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

        # rustfmt.toml uses imports_granularity = Module, which is nightly-only,
        # so `just fmt` needs a nightly rustfmt even though the rest of the
        # toolchain is stable.
        nightlyRustfmt = fenix.packages.${system}.complete.rustfmt;
      in {
        default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.rust-analyzer
            pkgs.just

            # mosquitto-rs builds OpenSSL from source (vendored-openssl) via
            # cmake + perl; libsqlite3-sys (rusqlite bundled) compiles sqlite.
            pkgs.cmake
            pkgs.perl
            pkgs.pkg-config
          ];

          # `cargo fmt` shells out to this rustfmt; point it at the nightly one.
          RUSTFMT = "${nightlyRustfmt}/bin/rustfmt";
        };
      }
    );

    formatter = forAllSystems ({pkgs, ...}: pkgs.alejandra);
  };
}
