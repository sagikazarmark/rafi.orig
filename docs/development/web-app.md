# The web app

Notes for working on the browser side: `apps/web` (admin, projector, verification page) and `apps/participant`
(the participant page's script).

## Two front ends

- **`rafi-web`** (`apps/web`) is a Dioxus 0.7 single-page app, built by `dx`. It calls the server's JSON routes on
  the same origin: `POST /api/admin/<function>` for the admin, and the public views. Both sides of the admin API come
  from one declaration in `rafi-api` (`admin_functions!`), so a function is added in one place.
- **The participant page** is server-rendered by `rafi-server` (`/j/<CODE>`), so it works on poor venue wifi. A small
  WebAssembly module (`apps/participant`, a Dioxus binary `dx` builds through its `justfile`) keeps it
  live, with a JavaScript fallback (`public/fallback.js`). The build names the module and its glue by their hash,
  so phones keep them for good, and fills them into `index.html`, the page's loader, which `rafi-server` compiles
  in: a server is built against one participant build. What a phone shows is a pure function of the participant
  view and its own entrant ID (`rafi-live-client`).

## Content-Security-Policy

Every page is served with a Content-Security-Policy ([routes](../reference/routes.md#security-headers)), so a
script has to come from the server's origin (or Clerk's), never inline:

- `apps/web/index.html` has no inline script: the server refuses to serve one that does (`Shell::new`). What has to
  run before first paint goes into `/config.js` (`rafi-server`'s `public::config_js`), as the saved theme does.
- The server's pages carry two inline scripts, the head script and the participant build's loader, allowed by their
  hashes, which the server computes from what it compiled in; `csp_by_page` checks every inline script a page has is
  among them. A new inline script on a server page fails that test until it is in `pages::inline_scripts`.
- The web app's policy allows `'unsafe-eval'`, which Dioxus's `document::eval` (and `document::Title`) need, and
  inline styles. The participant page's allows neither: its module and `fallback.js` don't use them.
- `just workers` runs the evening with every tab listening for violations; `RAFI_CSP=report-only` turns a policy
  into reports in the browser's console while finding out what one would break.

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
  store is the `pow_used` table.

Both are developed in [altcha-rs](https://github.com/sagikazarmark/altcha-rs). The server serves the solver at
`/participant/pow-worker.js` from `dioxus_altcha::WORKER_JS`, so it always matches the version built in.
