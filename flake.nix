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
        rustToolchain = fenix.packages.${system}.stable.toolchain;

        nightlyRustfmt = fenix.packages.${system}.complete.rustfmt;
      in {
        default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.rust-analyzer
            pkgs.just

            pkgs.cmake
            pkgs.perl
            pkgs.pkg-config
          ];

          # `cargo fmt` shells out to this rustfmt; point it at the nightly one.
          RUSTFMT = "${nightlyRustfmt}/bin/rustfmt";
        };

        # reverse-engineering toolchain
        re = pkgs.mkShell {
          packages = [
            pkgs.mitmproxy # capture the app's HTTPS/MQTT traffic
            pkgs.jadx # dex -> java for the apk splits
            pkgs.apktool # decode resources + AndroidManifest
            pkgs.unzip # extract .so / split apks
          ];
        };
      }
    );

    formatter = forAllSystems ({pkgs, ...}: pkgs.alejandra);
  };
}
