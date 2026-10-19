{ pkgs, config, ... }:

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
    cargo-release
    cargo-watch

    # Rafi
    python3 # tools/verify.py
    sqlite
    wrangler # Cloudflare Workers: wrangler dev, D1 migrations, deploys

    scaleway-cli
  ];

  dioxus = {
    enable = true;
  };

  worker-build = {
    enable = true;
  };

  languages = {
    rust = {
      enable = true;
      channel = "stable";
      targets = [ "wasm32-unknown-unknown" ];
    };

    # daisyUI for apps/web/tailwind.css, installed on entering the shell.
    javascript = {
      enable = true;
      directory = "${config.devenv.root}/apps/web";
      npm.enable = true;
      npm.install.enable = true;
    };

    # dx (built without downloads) runs this tailwindcss.
    tailwindcss = {
      enable = true;
    };
  };

  # dx must match the dioxus version exactly (0.7.10):
  #   cargo install dioxus-cli@0.7.10 --locked
  # Local Restate server for development: $(./scripts/restate-server.sh) (see the README)
}
