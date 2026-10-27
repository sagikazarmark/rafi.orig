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
#   devenv --profile clerk up     # with Clerk instead of dev auth (combines with the others)

let
  cfg = config.rafi;

  root = config.devenv.root;
  state = config.devenv.state;
  port = name: config.processes.${name}.ports.http.value;

  restateIngressUrl = "http://127.0.0.1:${toString (port "restate")}";

  # The app and the engine write the same SQLite file.
  databaseUrl = "sqlite://${state}/rafi.db";

  # Dev auth, or Clerk (the clerk profile) with the keys from .env. The web app
  # reads CLERK_PUBLISHABLE_KEY at build time: empty, it uses dev auth.
  noClerk = {
    CLERK_PUBLISHABLE_KEY = "";
  };
  requireClerkKeys = lib.optionalString cfg.clerk ''
    : "''${CLERK_PUBLISHABLE_KEY:?the clerk profile needs CLERK_PUBLISHABLE_KEY (in .env)}"
    : "''${CLERK_SECRET_KEY:?the clerk profile needs CLERK_SECRET_KEY (in .env)}"
  '';

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
  # The server allows RAFI_DEV_AUTH only when built with `dev`.
  rafiFeatures = " --features dev" + lib.optionalString cfg.mockDrand ",mock-drand";

  # The Workers' static files (apps/worker/worker-assets) are rebuilt when their
  # sources change, when something else rebuilt them (scripts/workers-check.sh),
  # or for the other auth: the stamp records the build.
  appAssets = "apps/worker/worker-assets";
  appAssetsStamp = "target/app-assets.auth";
  appAssetsAuth = if cfg.clerk then "clerk" else "dev";
  appAssetsSources = [
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

    clerk = lib.mkEnableOption "Clerk instead of dev auth, with CLERK_PUBLISHABLE_KEY and CLERK_SECRET_KEY from .env";
  };

  config = lib.mkMerge [
    {
      dotenv.enable = true;

      packages = with pkgs; [
        cargo-deny

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

      browsers = {
        chrome.enable = true;
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

        clerk.module = {
          rafi.clerk = true;
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
        exec = ''
          ${requireClerkKeys}
          exec cargo run -p rafi${rafiFeatures}
        '';
        cwd = root;
        env = {
          RAFI_PUBLIC_URL = "http://localhost:${toString (port "dioxus")}";
          RAFI_DATABASE_URL = databaseUrl;
          RAFI_ENGINE_INGRESS_URL = restateIngressUrl;
          PORT = "8788";
        }
        // lib.optionalAttrs (!cfg.clerk) {
          RAFI_DEV_AUTH = "1";
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
          # Loopback, Restate alone: no request signing in development.
          RAFI_RESTATE_ENDPOINT__INSECURE = "true";
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
      processes.dioxus.env = lib.mkIf (!cfg.clerk) noClerk;
    })

    (lib.mkIf (cfg.stack == "workers") {
      # The Worker under `wrangler dev --env dev`, its local data (D1, Durable
      # Objects) kept across restarts, the D1 migrations applied before it starts.
      wrangler.dev.app = {
        directory = "apps/worker";
        environment = "dev";
        d1.migrate = [ "DB" ];
        # Ready once the app answers, not when wrangler listens.
        ready.path = "/healthz";
        vars = {
          RAFI_PUBLIC_URL = config.wrangler.dev.app.url;
          RAFI_ENGINE_INGRESS_URL = restateIngressUrl;
        }
        # The dev environment's RAFI_DEV_AUTH off: Clerk signs in.
        // lib.optionalAttrs cfg.clerk { RAFI_DEV_AUTH = "0"; };
        # The secret key from .env.
        extraArgs = lib.optionals cfg.clerk [
          "--env-file"
          "${root}/.env"
        ];
      };

      # The raffle engine is the same Worker, under /restate.
      services.restate.deployments.engine = {
        uri = "${config.wrangler.dev.app.url}/restate";
        http1 = true;
        after = [ "devenv:processes:wrangler-app" ];
      };

      tasks = {
        # The web app bundle and the participant scripts -> apps/worker/worker-assets.
        "rafi:app-assets" = {
          exec = ''
            ${requireClerkKeys}
            ./scripts/app-assets.sh${lib.optionalString (!cfg.clerk) " --dev"}
            mkdir -p "$(dirname ${appAssetsStamp})"
            echo ${appAssetsAuth} > ${appAssetsStamp}
          '';
          status = ''
            [ "$(cat ${appAssetsStamp} 2>/dev/null)" = ${appAssetsAuth} ] \
              && [ -f ${appAssets}/index.html ] \
              && [ -z "$(find ${appAssets}/index.html ${lib.concatStringsSep " " appAssetsSources} \
                -newer ${appAssetsStamp} ! -path apps/web/assets/tailwind.css -print -quit)" ]
          '';
          cwd = root;
          before = [ "devenv:processes:wrangler-app" ];
        };
      };
    })
  ];
}
