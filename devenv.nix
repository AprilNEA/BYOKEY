{ pkgs, ... }:

{
  languages.rust.enable = true;
  packages = [
    pkgs.protobuf
    pkgs.nodejs
    pkgs.biome
  ];
}
