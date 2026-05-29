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

            # ui/ build chain: vite + svelte 5 + tailwind 4 + bits-ui.
            # corepack enables the pinned pnpm shipped with node.
            pkgs.nodejs_24
            pkgs.pnpm
          ];

          # btleplug's Linux backend builds against system libdbus (BlueZ lives on
          # the system bus); pkg-config finds it via buildInputs, and the built
          # binary needs libdbus-1.so on the runtime linker path too.
          buildInputs = [pkgs.dbus];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [pkgs.dbus];

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
            # tshark parses BLE btsnoop captures; the decrypt-btsnoop.py tool
            # shells out to it. python3 has cryptography for the V1 Safe decrypt.
            pkgs.wireshark-cli
            (pkgs.python3.withPackages (ps: [ps.cryptography]))
          ];
        };
      }
    );

    formatter = forAllSystems ({pkgs, ...}: pkgs.alejandra);
  };
}
