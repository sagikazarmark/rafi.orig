{ pkgs, config, ... }:

{
  dotenv.enable = true;

  packages = with pkgs; [
    cargo-deny
    cargo-watch

    # Rafi
    python3 # tools/verify.py
    sqlite
    wrangler # Cloudflare Workers: wrangler dev, D1 migrations, deploys
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

  # Local Restate server for development: $(./scripts/restate-server.sh) (see the README)
}
