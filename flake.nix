{
  description = "sifter - local-first search for code and docs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [(import rust-overlay)];
        pkgs = import nixpkgs {inherit system overlays;};

        stableToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["clippy" "llvm-tools-preview" "rust-src" "rustfmt"];
        };

        nightlyToolchain = pkgs.rust-bin.selectLatestNightlyWith (toolchain:
          toolchain.default.override {
            extensions = ["clippy" "llvm-tools-preview" "miri" "rust-src" "rustfmt"];
          });

        sifterPkg = pkgs.callPackage ./package.nix {};
      in {
        packages.sifter = sifterPkg;
        packages.default = sifterPkg;

        apps.sifter = flake-utils.lib.mkApp {
          drv = sifterPkg;
        };
        apps.default = self.apps.${system}.sifter;

        devShells.default = pkgs.mkShell {
          packages = [
            stableToolchain
            pkgs.cargo-audit
            pkgs.cargo-deny
            pkgs.cargo-edit
            pkgs.cargo-hack
            pkgs.cargo-llvm-cov
            pkgs.cargo-mutants
            pkgs.cargo-nextest
            pkgs.cargo-outdated
            pkgs.pkg-config
            pkgs.sqlite
          ];
        };

        devShells.nightly = pkgs.mkShell {
          packages = [
            nightlyToolchain
            pkgs.cargo-audit
            pkgs.cargo-deny
            pkgs.cargo-edit
            pkgs.cargo-hack
            pkgs.cargo-llvm-cov
            pkgs.cargo-mutants
            pkgs.cargo-nextest
            pkgs.cargo-outdated
            pkgs.cargo-udeps
            pkgs.pkg-config
            pkgs.sqlite
          ];
        };
      });
}
