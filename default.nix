# Standalone test harness. Run `nix-build` from this directory to verify the package builds.
let
  pkgs = import <nixpkgs> {};
in
pkgs.callPackage ./package.nix {}
