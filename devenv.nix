{ pkgs, ... }:

{
  dotenv.enable = true;

  dagger = {
    enable = true;
    version = "v1.0.0-beta.13";

    dang.enable = true;
  };

  packages = with pkgs; [
    lld
    cargo-audit
    cargo-deny
    cargo-dist
    cargo-release
    cargo-watch

    # Rafi
    python3 # tools/verify.py
    sqlite

    scaleway-cli
  ];

  languages = {
    rust = {
      enable = true;
      channel = "stable";
      targets = [ "wasm32-unknown-unknown" ];
    };
  };

  # dx must match the dioxus version exactly (0.7.10):
  #   cargo install dioxus-cli@0.7.10 --locked
  # Local Restate server for development: docker compose up -d (see the README)
}
