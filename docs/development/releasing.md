# Releasing

A release is a tag, `v<version>`; `.github/workflows/release.yaml` does the rest. What ships is the Cloudflare
release, one asset, `worker.tar.gz`: the Worker, the static files and the D1 migrations (`just
cloudflare-artifact`), at the top level of the archive. Self-hosting builds from the tag.

## Once

In the repository's settings (**Settings > General > Releases**), enable **immutable releases**: a published
release's assets and its tag can't change after publishing, so `worker.tar.gz` under a version is the one GitHub
showed the digest of and attested. Installations download it by URL and check nothing else.

## Before the tag

On `main`, in one pull request:

1. **The version**: `config.rafi_version` in [`deploy/cloudflare/package.json`](../../deploy/cloudflare/package.json),
   the release the template downloads.
2. **The Deploy to Cloudflare buttons** name the tag: `tree/v<version>/deploy/cloudflare` in
   `deploy/cloudflare/README.md` and `docs/operations/cloudflare.md`, and the README's "the release this README is
   from".
The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act (a migration, a changed engine path, a new secret).

`scripts/cloudflare-release.sh` refuses a version that `package.json` doesn't pin, or buttons that name another tag,
so CI's `workers` job (which rehearses the pinned version on every push) fails until they agree.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default the one `package.json` pins) and deploys it (dry) through a copy of `deploy/cloudflare`,
publishing nothing. `dry_run` off publishes a tag that already exists, to finish a release whose run failed.

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

A version with a pre-release suffix (`v0.2.0-rc.1`) is published as a pre-release: pin it in `package.json` like any
other, and point the buttons back at the last release afterwards if it isn't followed by one.

## Reproducible

The tarball is packed reproducibly (`just release` in `apps/worker`, with GNU tar: entries sorted, every file's
time `SOURCE_DATE_EPOCH` (the commit's when unset), no owner, gzip without a name or a time), so two builds of one commit can be compared byte for byte:

```sh
just cloudflare-release <version>              # at the tag
sha256sum target/cloudflare/worker.tar.gz
```

The archive is deterministic; whether the WebAssembly inside is the same from another machine depends on the
toolchain and the paths it embeds, which the devenv shell pins.

## After

- `/healthz` on an installation answers `ok <version>`.
- The template on `main` now names the new release; installations upgrade by changing `config.rafi_version` in
  their `package.json` ([Upgrading](../../deploy/cloudflare/README.md#upgrading)).
