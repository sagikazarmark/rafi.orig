# Releasing

A release is a tag, `v<version>`; `.github/workflows/release.yaml` does the rest. What ships is the Cloudflare
release, one asset, `worker.tar.gz` (`just cloudflare-artifact`, packed by `just cloudflare-tarball`), with at
the top level of the archive:

- `VERSION`;
- `app/`: the app Worker (`apps/worker-app`), `build/` and its static files, `assets/` (without `legal/`, an
  installation's);
- `engine/`: the raffle engine Worker (`apps/worker-engine`), `build/`;
- `migrations/sqlite/`: the D1 migrations;
- `deploy.sh`: the deploy itself (`scripts/cloudflare-deploy.sh`; its header says what it does), so an installation
  deploys a release with that release's own logic, attested with it.

The combined Worker (`apps/worker`) is for local development only. Self-hosting builds from the tag.

Installations deploy a release with GitHub Actions: their copy of `deploy/cloudflare` has
`.github/workflows/deploy.yml`, which calls this repository's `.github/workflows/deploy-cloudflare.yml` at the
release's tag. That workflow downloads `worker.tar.gz`, verifies its attestation (signed by `release.yaml`, from a
run on the tag `v<version>`), and runs its `deploy.sh`. So the tag carries both the release and the workflow that
deploys it, and an installation that copies `deploy/cloudflare` at the tag deploys that tag.

## Once

In the repository's settings (**Settings > General > Releases**), enable **immutable releases**: a published
release's assets and its tag can't change after publishing, so `worker.tar.gz` under a version is the one GitHub
showed the digest of and attested, and the tag `v<version>` that installations' workflows call
`deploy-cloudflare.yml` at stays the commit it was. Installations' deploys download the asset by its tag and
verify its attestation.

## Before the tag

Name the release in `deploy/cloudflare/.github/workflows/deploy.yml`, in both places: the reusable workflow's ref,
`uses: sagikazarmark/rafi/.github/workflows/deploy-cloudflare.yml@v<version>`, and `version: "<version>"`. The
rehearsal refuses a release that file doesn't name (an installation copied at the tag would deploy another
release), and CI's rehearsal is of `apps/worker-app`'s crate version, so bump the crate version and the template's
workflow together, for a pre-release too (`0.2.0-rc.1` in all three).

The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act (a migration, a new secret, a setting in `deploy/cloudflare`'s configurations or its workflow, a change to
`deploy-cloudflare.yml`'s inputs or secrets), and say when a migration or the live channel would break the previous
release's engine while it drains ([Upgrades](../operations/upgrades.md#what-a-deploy-does-to-a-raffle-in-flight)).

CI's `workers` job rehearses a release (of `apps/worker-app`'s crate version) on every push.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default `apps/worker-app`'s crate version), checks that `deploy/cloudflare`'s workflow names it,
and runs its `deploy.sh --dry-run` from a copy of `deploy/cloudflare` with the release unpacked into `release/`,
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
`rafi-engine-0-2-0-rc-1`.

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
- Installations upgrade by naming it in their `.github/workflows/deploy.yml` (both places) and pushing; their
  deploy verifies the attestation and runs the release's own `deploy.sh`. A release's engine is the Worker
  `rafi-engine-<version, dashed>`, registered with each installation's Restate by its deploy, and the previous
  release's engine is pruned once drained.
