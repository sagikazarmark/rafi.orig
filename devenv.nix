{
  pkgs,
  lib,
  config,
  ...
}:

# `devenv up` runs Rafi locally, with live reload (see docs/development/getting-started.md):
#
#   devenv up                     # natively: Restate, the engine, the server and `dx serve`
#   devenv --profile mock up      # the same, offline: the mock drand chain
#   devenv --profile workers up   # on Cloudflare Workers, under `wrangler dev`

let
  cfg = config.rafi;

  root = config.devenv.root;
  state = config.devenv.state;
  port = name: config.processes.${name}.ports.http.value;

  restateIngressUrl = "http://127.0.0.1:${toString (port "restate")}";

  # The app and the engine write the same SQLite file.
  databaseUrl = "sqlite://${state}/rafi.db";

  # The stacks run with dev auth. The web app reads CLERK_PUBLISHABLE_KEY
  # (perhaps from .env) at build time: empty, it uses dev auth too.
  noClerk = {
    CLERK_PUBLISHABLE_KEY = "";
  };

  # `cargo run`, restarted when the Rust code changes.
  rustWatch = app: {
    paths = [
      app
      ./crates
      ./migrations
      ./Cargo.lock
    ];
    extensions = [
      "rs"
      "toml"
      "sql"
      "lock"
    ];
  };

  features = lib.optionalString cfg.mockDrand " --features mock-drand";

  # Local state shared by both Workers (D1, Durable Objects).
  wranglerState = "${state}/wrangler/state";
in
{
  options.rafi = {
    stack = lib.mkOption {
      type = lib.types.enum [
        "native"
        "workers"
      ];
      default = "native";
      description = "What `devenv up` runs: the native binaries, or the Workers under `wrangler dev`.";
    };

    mockDrand = lib.mkEnableOption "the mock drand chain (offline; draws are not verifiable)";
  };

  config = lib.mkMerge [
    {
      dotenv.enable = true;

      packages = with pkgs; [
        cargo-deny
        cargo-watch

        # Rafi
        python3 # tools/verify.py
        sqlite
      ];

      dioxus = {
        enable = true;
      };

      worker-build = {
        enable = true;
      };

      # Cloudflare Workers: wrangler dev, D1 migrations, deploys
      wrangler = {
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
          directory = "${root}/apps/web";
          npm.enable = true;
          npm.install.enable = true;
        };

        # dx (built without downloads) runs this tailwindcss.
        tailwindcss = {
          enable = true;
        };
      };

      services.restate = {
        enable = true;
        package = pkgs.restate-bin;
      };

      # The end-to-end tests start their own restate-server.
      env.RESTATE_SERVER_BIN = lib.getExe' config.services.restate.package "restate-server";

      profiles = {
        mock.module = {
          rafi.mockDrand = true;
        };

        workers.module = {
          rafi.stack = "workers";
        };
      };
    }

    (lib.mkIf (cfg.stack == "native") {
      # The participant page's WebAssembly and fallback (apps/participant/dist),
      # rebuilt when they change. The server serves the directory as it is.
      processes.participant = {
        exec = "${root}/scripts/participant.sh";
        cwd = root;
        # Not apps/participant itself: the build writes apps/participant/dist.
        watch.paths = [
          ./apps/participant/src
          ./apps/participant/fallback.js
          ./apps/participant/Cargo.toml
          ./crates/live-client
        ];
        restart.on = "never";
      };

      # The server. `dx serve` proxies the server's paths to it on :8788 (apps/web/Dioxus.toml).
      processes.rafi = {
        exec = "exec cargo run -p rafi${features}";
        cwd = root;
        env = {
          RAFI_DEV_AUTH = "1";
          RAFI_PUBLIC_URL = "http://localhost:${toString (port "dioxus")}";
          RAFI_DATABASE_URL = databaseUrl;
          RESTATE_INGRESS_URL = restateIngressUrl;
          PORT = "8788";
        }
        // lib.optionalAttrs cfg.mockDrand {
          RAFI_DRAND_CHAIN = "mock";
        };
        ready.exec = "curl -fsS -o /dev/null http://127.0.0.1:8788/healthz";
        watch = rustWatch ./apps/rafi;
      };

      # The raffle engine.
      processes.engine = {
        exec = "exec cargo run -p rafi-restate${features}";
        cwd = root;
        ports.http.allocate = 9080;
        env = {
          RAFI_RESTATE_ENDPOINT__LISTENER = "127.0.0.1:${toString (port "engine")}";
          RAFI_RESTATE_DATABASE__URL = databaseUrl;
        }
        // lib.optionalAttrs cfg.mockDrand {
          RAFI_RESTATE_DRAND__MOCK = "true";
        };
        watch = rustWatch ./apps/restate;
      };

      services.restate.deployments.engine = {
        uri = "http://127.0.0.1:${toString (port "engine")}";
        after = [ "devenv:processes:engine" ];
      };

      # The web app, with hot reload.
      dioxus.serve = {
        enable = true;
        directory = "apps/web";
        port = 8787;
      };
      processes.dioxus.env = noClerk;
    })

    (lib.mkIf (cfg.stack == "workers") {
      wrangler.dev = {
        app = {
          directory = "apps/worker";
          port = 8787;
          extraArgs = [
            "--env"
            "dev"
            "--persist-to"
            wranglerState
            "--var"
            "RAFI_PUBLIC_URL:http://127.0.0.1:${toString (port "wrangler-app")}"
            "--var"
            "RESTATE_INGRESS_URL:${restateIngressUrl}"
          ];
        };

        engine = {
          directory = "apps/restate-worker";
          port = 8791;
          extraArgs = [
            "--env"
            "dev"
            "--persist-to"
            wranglerState
          ];
        };
      };

      processes."wrangler-app".ready.exec =
        "curl -fsS -o /dev/null http://127.0.0.1:${toString (port "wrangler-app")}/healthz";
      processes."wrangler-engine".ready.exec =
        "curl -sS -o /dev/null http://127.0.0.1:${toString (port "wrangler-engine")}/discover";

      services.restate.deployments.engine = {
        uri = "http://127.0.0.1:${toString (port "wrangler-engine")}";
        http1 = true;
        after = [ "devenv:processes:wrangler-engine" ];
      };

      tasks = {
        # The web app bundle and the participant scripts -> apps/worker/worker-assets.
        "rafi:app-assets" = {
          exec = "CLERK_PUBLISHABLE_KEY= ${root}/scripts/app-assets.sh";
          cwd = root;
          execIfModified = [
            "apps/web/src"
            "apps/web/assets"
            "apps/web/tailwind.css"
            "apps/web/Cargo.toml"
            "apps/web/Dioxus.toml"
            "apps/participant/src"
            "apps/participant/fallback.js"
            "apps/participant/Cargo.toml"
            "crates"
            "Cargo.lock"
          ];
          before = [ "devenv:processes:wrangler-app" ];
        };

        # The database, shared by both Workers.
        "rafi:d1-migrations" = {
          exec = "wrangler d1 migrations apply rafi --local --env dev --persist-to ${wranglerState}";
          cwd = "${root}/apps/worker";
          before = [
            "devenv:processes:wrangler-app"
            "devenv:processes:wrangler-engine"
          ];
        };
      };
    })
  ];
}
