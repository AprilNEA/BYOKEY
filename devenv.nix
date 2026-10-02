{ pkgs, ... }:

{
  languages.rust.enable = true;
  packages = [ pkgs.protobuf ];
}
