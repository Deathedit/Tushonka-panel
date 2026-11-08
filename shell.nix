# Dev shell for NixOS / nix users: `nix-shell` then `make` (see README).
{ pkgs ? import <nixpkgs> { } }:
pkgs.mkShell {
  packages = with pkgs; [ cargo rustc clippy rustfmt rust-analyzer nodejs_22 gnumake ];
}
