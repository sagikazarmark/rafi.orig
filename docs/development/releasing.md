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

## Once

In the repository's settings (**Settings > General > Releases**), enable **immutable releases**: a published
release's assets and its tag can't change after publishing, so `worker.tar.gz` under a version is the one GitHub
showed the digest of and attested. Installations download it by URL and check nothing else.

## Before the tag

The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act (a migration, a new secret, a setting in `deploy/cloudflare`'s configuration).

CI's `workers` job rehearses a release (of `apps/worker-app`'s crate version) on every push.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default `apps/worker-app`'s crate version) and runs its `deploy.sh --dry-run` from a copy of
`deploy/cloudflare` with the release unpacked into `release/`, publishing nothing. `dry_run` off publishes a tag that already exists, to finish a release whose run failed.

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
3. attests the tarball's build provenance (`gh attestation verify worker.tar.gz -R <repository>` checks it);
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
- Installations deploy it with its own `deploy.sh`, from their copy of `deploy/cloudflare` with the release
  unpacked into `release/`.
