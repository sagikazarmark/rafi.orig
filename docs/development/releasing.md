# Releasing

A release is a tag, `v<version>`; `.github/workflows/release.yaml` does the rest. What ships is the Cloudflare
release, one asset, `worker.tar.gz` (`just cloudflare-artifact`, packed by `just cloudflare-tarball`), with at
the top level of the archive:

- `VERSION`;
- `DEPLOY_VERSION`: the deploy interface, an integer (the `DEPLOY_VERSION=` line in `scripts/cloudflare-deploy.sh`
  is its one source): how an installation's workflows drive `deploy.sh`;
- `app/`: the app Worker (`apps/worker-app`), `build/` and its static files, `assets/` (without `legal/`, an
  installation's);
- `engine/`: the raffle engine Worker (`apps/worker-engine`), `build/`;
- `rooms/`: the rooms Worker (`apps/worker-rooms`, the `RaffleRoom` Durable Objects), `build/`;
- `migrations/`: the D1 migrations (`migrations/sqlite`'s `.sql` files);
- `deploy.sh`: the deploy itself (`scripts/cloudflare-deploy.sh`; its header says what each subcommand does), so an
  installation deploys a release with that release's own logic, attested with it.

The combined Worker (`apps/worker`) is for local development only. Self-hosting builds from the tag.

Installations deploy a release with GitHub Actions, from their own repository, a copy of `deploy/cloudflare`: its
`VERSION` names the release, and its two workflows, `.github/workflows/deploy.yaml` and `restate-cleanup.yaml`,
download that release's `worker.tar.gz` from this repository, verify its attestation (signed by `release.yaml`,
from a run on the tag `v<version>`) and run its `deploy.sh`, one subcommand per job (`check`, `migrate`,
`deploy-rooms`, `upload-app` and `deploy-engine`, `register-engine` and `promote-app`; `cleanup` daily). The
workflows are thin and the installation's own; they say which deploy interface they are written for
(`RAFI_DEPLOY_VERSION`), and `deploy.sh` refuses to run under another than its release's `DEPLOY_VERSION`, naming
the template at the release's tag to copy them from. So an installation that copies `deploy/cloudflare` at a tag
deploys that tag, and upgrading is changing `VERSION` (and, when `DEPLOY_VERSION` changed, copying the workflows).

## Once

In the repository's settings (**Settings > General > Releases**), enable **immutable releases**: a published
release's assets and its tag can't change after publishing, so `worker.tar.gz` under a version is the one GitHub
showed the digest of and attested, and the tag `v<version>` an installation copies `deploy/cloudflare` at stays the
commit it was. Installations' deploys download the asset by its tag and verify its attestation.

## Before the tag

Name the release in `deploy/cloudflare/VERSION` (`0.2.0`, or `0.2.0-rc.1` for a pre-release). The rehearsal refuses
a release that file doesn't name (an installation copied at the tag would deploy another release), and CI's
rehearsal and the Release workflow's manual run are of the version it names. The template's README names the
release in its commands too (the tarball it copies, `/healthz`'s answer, the engine Worker's name): change them with
`VERSION` (`rg -n '0\.1\.0|0-1-0' deploy/cloudflare/README.md`); nothing checks those.

When a change to `scripts/cloudflare-deploy.sh` changes how it is driven (a subcommand, its environment or outputs,
what it expects of the installation's repository), increment its `DEPLOY_VERSION`, change the template's workflows
to fit and set their `RAFI_DEPLOY_VERSION` (both of them) to it: the rehearsal refuses workflows whose
`RAFI_DEPLOY_VERSION` isn't `deploy.sh`'s, and the release notes then tell operators to copy the workflows. The
template's README names it too, in its optional local check (`RAFI_DEPLOY_VERSION=1 release/deploy.sh check`):
change it there.

The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act or should know:

- a migration, a new secret, a setting in `deploy/cloudflare`'s configurations or `.prod.vars.example`;
- a new `DEPLOY_VERSION`: "copy `.github/workflows/deploy.yaml` and `restate-cleanup.yaml` from
  `deploy/cloudflare` at this tag before changing `VERSION`" (the deploy's check says the same), or a change to the
  workflows' secrets;
- whether the rooms Worker changed (its `rooms/build` differs from the previous release's): a deploy of a release
  that changes it drops every live connection, so say so;
- when a migration or the live channel would break the previous release's engine while it drains
  ([Upgrades](../operations/upgrades.md#what-a-deploy-does-to-a-raffle-in-flight)), or a change to the rooms
  Worker's requests would break the previous app's or engine's (a change to them takes two releases).

CI's `workers` job rehearses a release (of the version `deploy/cloudflare/VERSION` names) on every push.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default the one `deploy/cloudflare/VERSION` names), checks that the template's `VERSION` names it
and that its workflows are written for this `deploy.sh` (`RAFI_DEPLOY_VERSION`), and runs its `deploy.sh dry-run`
from a copy of `deploy/cloudflare` with the release unpacked into `release/`,
publishing nothing. `dry_run` off publishes a tag that already exists, to finish a release whose run failed: run it
on the tag (**Use workflow from**: the tag, or `gh workflow run release.yaml --ref v<version> -f dry_run=false`),
since the attestation names the ref the run was on and installations require the tag; from a branch it refuses.

Locally, `just cloudflare-release <version>` builds and rehearses the same, into `target/cloudflare`.

## The tag

```sh
git tag -s v<version> -m "Rafi <version>"
git push origin v<version>
```

The workflow then:

1. runs CI on the tag's commit (`ci.yaml`, called): if anything fails, the release job is skipped, and nothing is
   published;
2. builds and rehearses the release (`just cloudflare-release`);
3. attests the tarball's build provenance, signed as `.github/workflows/release.yaml@refs/tags/v<version>`
   (`gh attestation verify worker.tar.gz -R <repository> --signer-workflow
   github.com/<repository>/.github/workflows/release.yaml --source-ref refs/tags/v<version>` checks it, as every
   installation's deploy does);
4. creates the release as a **draft**, with `worker.tar.gz` its one asset;
5. checks the digest GitHub computed for the uploaded tarball (the one the release page shows) against the build's
   own `sha256sum`; on a mismatch the release stays a draft;
6. publishes it.

A version with a pre-release suffix (`v0.2.0-rc.1`) is published as a pre-release; its engine Worker is
`rafi-engine-v0-2-0-rc-1`.

## Reproducible

The tarball is packed reproducibly (`just cloudflare-tarball`, with GNU tar: entries sorted, every file's
time `SOURCE_DATE_EPOCH` (the commit's when unset), no owner, modes 644 and 755, gzip without a name or a time), so two builds of one commit can be compared byte for byte:

```sh
just cloudflare-release <version>              # at the tag
sha256sum target/cloudflare/worker.tar.gz
```

The archive is deterministic; whether the WebAssembly inside is the same from another machine depends on the
toolchain and the paths it embeds, which the devenv shell pins.

## After

- `/healthz` on an installation answers `ok <version>`.
- Installations upgrade by naming it in their `VERSION` and pushing (copying the template's workflows at the tag
  first when `DEPLOY_VERSION` changed); their deploy verifies the attestation and runs the release's own
  `deploy.sh`. A release's engine is the Worker `rafi-engine-v<version, dashed>`, registered with each installation's
  Restate by its deploy (never over a newer release's), and the previous release's engine Worker is deleted by the
  installation's Restate cleanup workflow once Restate reports it drained.
