# The web app

Notes for working on the browser side: `apps/web` (admin, projector, verification page) and `apps/participant`
(the participant page's script).

## Two front ends

- **`rafi-web`** (`apps/web`) is a Dioxus 0.7 single-page app, built by `dx`. It calls the server's JSON routes on
  the same origin: `POST /api/admin/<function>` for the admin, and the public views. Both sides of the admin API come
  from one declaration in `rafi-api` (`admin_functions!`), so a function is added in one place.
- **The participant page** is server-rendered by `rafi-server` (`/j/<CODE>`), so it works on poor venue wifi. A small
  WebAssembly module (`apps/participant`, a Dioxus binary `dx` builds through `./scripts/participant.sh`) keeps it
  live, with a JavaScript fallback (`public/fallback.js`). The build names the module and its glue by their hash,
  so phones keep them for good, and fills them into `index.html`, the page's loader, which `rafi-server` compiles
  in: a server is built against one participant build. What a phone shows is a pure function of the participant
  view and its own entrant ID (`rafi-live-client`).

## Styling

The UI uses **Tailwind CSS v4 and daisyUI 5**, with custom light (`rafi`) and dark (`rafi-dark`) themes defined in
[`apps/web/tailwind.css`](../../apps/web/tailwind.css). `dx` compiles it automatically with its own standalone
Tailwind CLI. daisyUI comes from npm (`apps/web/package.json`). A plain `cargo build` or `clippy` of the web app
needs the compiled stylesheet too: on a fresh checkout, run `dx build` once, or
`tailwindcss -i tailwind.css -o assets/tailwind.css` in `apps/web`.

## Forms and components

Organizer forms use [dioform](https://crates.io/crates/dioform) (typed fields, validation, managed async submit).
They are rendered with components from the
[dioxus-daisyui-components](https://github.com/sagikazarmark/dioxus-daisyui-components) registry: input and textarea
fields, switch, buttons, alert dialogs, dropdown menu, tabs and toasts. The components are installed into
`apps/web/src/components/` with:

```sh
dx components add <name> --path <registry checkout>
```

**Treat `apps/web/src/components/` as vendored**: update it by re-installing (`--force`), not by editing.

Icons are `dioxus-icons`' Lucide components.

## The UI derives no rules

The server sends the admin what it may do next (`RaffleAdmin.actions`, `AdminEntrant.deliverable`, …). The rules
live in `rafi-raffle` and `rafi-lifecycle`. When a button needs a condition, add it to the server's answer rather
than computing it in the UI.

## The proof-of-work

Joining uses ALTCHA v2:

- [dioxus-altcha](https://crates.io/crates/dioxus-altcha) solves it in the browser (Web Workers, WebCrypto)
- [altcha-server](https://crates.io/crates/altcha-server) issues and redeems challenges on the server. Rafi's replay
  store is the `altcha_used` table.

Both are developed in [altcha-rs](https://github.com/sagikazarmark/altcha-rs). The server serves the solver at
`/participant/altcha-worker.js` from `dioxus_altcha::WORKER_JS`, so it always matches the version built in.
