# Releasing

A release is a tag, `v<version>`; `.github/workflows/release.yaml` does the rest. What ships is the Cloudflare
release, `rafi-cloudflare-<version>.tar.gz` with its `SHA256SUMS`: the Worker, the static files and the D1
migrations (`just cloudflare-artifact`). Self-hosting builds from the tag.

## Before the tag

On `main`, in one pull request:

1. **The version**: `RAFI_VERSION=<version>` in [`deploy/cloudflare/deploy.sh`](../../deploy/cloudflare/deploy.sh)
   (`RAFI_SHA256` stays empty: the template can't know the hash of a release not yet built).
2. **The Deploy to Cloudflare buttons** name the tag: `tree/v<version>/deploy/cloudflare` in
   `deploy/cloudflare/README.md` and `docs/operations/cloudflare.md`, and the README's "the release this README is
   from".
The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act (a migration, a changed engine path, a new secret).

`scripts/cloudflare-release.sh` refuses a version that `deploy.sh` doesn't pin, or buttons that name another tag,
so CI's `workers` job (which rehearses the pinned version on every push) fails until they agree.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default the one `deploy.sh` pins) and deploys it (dry) through a copy of `deploy/cloudflare`,
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
3. attests the tarball's build provenance (`gh attestation verify rafi-cloudflare-<version>.tar.gz -R <repository>`
   checks it);
4. creates the release as a **draft**, with the tarball and `SHA256SUMS`;
5. checks the digest GitHub computed for the uploaded tarball against the build's (what installations pin as
   `RAFI_SHA256`); on a mismatch the release stays a draft;
6. publishes it.

A version with a pre-release suffix (`v0.2.0-rc.1`) is published as a pre-release: pin it in `deploy.sh` like any
other, and point the buttons back at the last release afterwards if it isn't followed by one.

## Reproducible

The tarball is packed reproducibly (`just release` in `apps/worker`, with GNU tar: entries sorted, every file's
time `SOURCE_DATE_EPOCH` (the commit's when unset), no owner, gzip without a name or a time), so two builds of one commit can be compared byte for byte:

```sh
just cloudflare-release <version>              # at the tag
sha256sum target/cloudflare/rafi-cloudflare-<version>.tar.gz
```

The archive is deterministic; whether the WebAssembly inside is the same from another machine depends on the
toolchain and the paths it embeds, which the devenv shell pins.

## After

- `/healthz` on an installation answers `ok <version>`.
- The template on `main` now names the new release; installations upgrade by changing `RAFI_VERSION` and
  `RAFI_SHA256` ([Upgrading](../../deploy/cloudflare/README.md#upgrading)).
