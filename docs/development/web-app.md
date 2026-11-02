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
  hashes, which the server computes from what it compiled in; `server_pages_allow_their_inline_scripts_by_hash`
  checks every inline script a page has is among them. A new inline script on a server page fails that test until it
  is in `pages::embedded_scripts`.
- The operator's Markdown (`/privacy`, `/terms`) is rendered without `style` attributes: tables' columns aren't
  aligned.
- The web app's policy allows `'unsafe-eval'`, which Dioxus's `document::eval` (and `document::Title`) need, and
  inline styles. The participant page's allows neither: its module and `fallback.js` don't use them.
- `just workers` runs the evening with every tab listening for violations; `RAFI_CSP=report-only` turns a policy
  into reports in the browser's console while finding out what one would break.
- `frame-ancestors` holds for the page a frame loaded, not for where the router takes it next, so the admin's layout
  (`AdminLayout`) renders nothing of the admin inside a frame (`window.top !== window.self`), Clerk included, only a
  link that opens it at the top. A new admin route goes under that layout.
  `the_admin_does_not_render_in_a_frame` renders the router at the admin's paths, natively, as if framed.

## Styling

The UI uses **Tailwind CSS v4 and daisyUI 5**, with custom light (`rafi`) and dark (`rafi-dark`) themes defined in
[`apps/web/tailwind.css`](../../apps/web/tailwind.css). `dx` compiles it automatically with its own standalone
Tailwind CLI. daisyUI comes from npm (`apps/web/package.json`). A plain `cargo build` or `clippy` of the web app
needs the compiled stylesheet too: on a fresh checkout, run `just css` (which `just clippy` and `just test` do
first).

Every colour and its `-content`, and `base-content` on each base, reads at 4.5:1 or more in both themes, and so do
the stops of the light theme's `.gradient-text` (the winner's name): `theme_contrast` in `rafi-web` checks the
stylesheet, so a colour that doesn't fails `cargo test -p rafi-web`.

## Following a raffle live

- **The projector and the admin** poll the full view (`ui/live.rs`, `LiveView`). A countdown reads the server's
  time through `ServerClock`, the offset between the view's `server_now_ms` and the browser's clock taken when the
  view arrived, not recomputed each render (which would pin it to the view's own time). Failed fetches are counted
  (`Trouble`): before any view, the projector says there is no raffle at the address (a 404) or that the server
  can't be reached; over one, a "Reconnecting…" badge after ten seconds. A reply the page can't read
  (`codes::UNREADABLE`) is a page older than the server: the projector reloads, once a minute at most. The admin's
  header shows "Offline" from `navigator.onLine` and the same count. Polls are jittered (±20 %), back off while they
  fail (doubling to 10 s), and stop while the page is hidden; the page coming back fetches at once (one fetch for
  the events that come together). `Follow::Poll { every_ms }` is the server's knob for slowing every screen down.
- **The admin's data** (`get_raffle`, every entrant) is fetched again when the live view moves: at once for a moment
  of the evening (the state, a draw pending or decided, a failure, the channel ended), otherwise at most every
  10 s, so a room joining doesn't refetch the list with each join.
- **The projector** is dark unless a scheme was picked with its toggle (a lit room washes out a light screen); its
  type is in `em` of a root that grows with the screen, chips included (no daisyUI badges, which are rem-sized).
- **Phones** follow the live channel; one the server refuses or closes for good (4403) polls the participant view
  instead, and tries the channel again after 5 s, doubling to a minute. A WebAssembly module that arrives after the
  loader started `fallback.js` doesn't start: the fallback has the page.

## What can't be taken back

Every action that deletes, erases, cuts people off or can't be undone asks first or offers an undo:

| Action | Where | Guard |
|---|---|---|
| Delete a raffle | the header's menu | a confirmation naming what goes |
| Erase personal data now | the header's menu | a confirmation |
| All prizes delivered | the header's menu | a confirmation (their data is then erased with everyone's) |
| Freeze and start the draw | the header | a confirmation (what it publishes, the list reminder) |
| Close registration | the header | a toast saying how to reopen; Reopen registration is the next button |
| Unpublish | the header | a confirmation once anyone is in |
| End live updates | the header | a confirmation |
| Remove a prize | the Prizes tab | a confirmation |
| Remove or erase an entrant | the People tab | a confirmation (at their request or not) |
| Took themselves off, and its undo | the People tab | a confirmation each way |
| Mark delivered | the People tab | an Undo beside "✓ delivered"; past the erasure date, a confirmation (they are erased within the hour) |
| Undo last add | the People tab | a confirmation |
| Not here, Declined, Withdraw, Abandon, Finish | the draw console | a confirmation each |

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

Icons are `dioxus-icons`' Lucide components. Emoji are decoration only: the big glyphs of empty states, the
projector's hero moments and the phone's celebration. Anything that means something (a state, an item's status, a
delivery, a button, a toast's title) is a Lucide icon or words.

Every page links the favicon (`assets/favicon.svg`) and the home-screen icon; the join page also has a link preview
(Open Graph: the raffle's title, a line, `assets/og.png`), so a join link pasted into a chat previews as the raffle.
`index.html` shows the brand and a loader while the WebAssembly downloads (`#boot`, hidden by the stylesheet once the
app renders into `#main`), and preloads the display font, as the server's pages do.

## The UI derives no rules

The server sends the admin what it may do next (`RaffleAdmin.actions`, `AdminEntrant.deliverable`, …). The rules
live in `rafi-raffle` and `rafi-lifecycle`. When a button needs a condition, add it to the server's answer rather
than computing it in the UI. The header's lifecycle buttons, for one, come from `actions.lifecycle` and
`actions.start`, which the server computes with the raffle's registration setting, and what blocks the next of
them from `actions.missing`; the draw console's "Confirmed: it's them" from `AdminEntrant.no_phone`; the
projector's "Come to the front!" from a draw's `no_phone`, and the freeze's reminder from the view's `list_added`.

## Adding a list

The list tab of Add people reads the organizer's list in the browser: `apps/web/src/import` (no Dioxus) parses it
(UTF-8, or UTF-16 with a byte order mark; the delimiter sniffed; RFC 4180 quotes; the limits), finds the columns,
and builds the preview with the rules `rafi_lifecycle::entrant` and `rafi_lifecycle::names` share with the server.
The UI (`ui/admin/list.rs`) holds the list in memory only and sends the preview's rows. The tests run natively
(`cargo test -p rafi-web`) on exports shaped like the common registration tools' in `apps/web/fixtures/import`,
their line ends and byte order marks kept as written (`.gitattributes`, `.editorconfig`). Supporting another
export starts with its fixture and a test.

## The proof-of-work

Joining uses ALTCHA v2:

- [dioxus-altcha](https://crates.io/crates/dioxus-altcha) solves it in the browser (Web Workers, WebCrypto)
- [altcha-server](https://crates.io/crates/altcha-server) issues and redeems challenges on the server. Rafi's replay
  store is the `pow_used` table.

Both are developed in [altcha-rs](https://github.com/sagikazarmark/altcha-rs). The server serves the solver at
`/participant/pow-worker.js` from `dioxus_altcha::WORKER_JS`, so it always matches the version built in.
