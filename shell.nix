{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  # Paquetes que estarán disponibles dentro de tu nix-shell
  buildInputs = [
    pkgs.git
    pkgs.cargo
    pkgs.rustc
  ];
