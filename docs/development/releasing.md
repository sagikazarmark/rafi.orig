# Releasing

A release is a tag, `v<version>`; `.github/workflows/release.yaml` does the rest. What ships is the Cloudflare
release, one asset, `worker.tar.gz` (`just cloudflare-artifact`, packed by `just cloudflare-tarball`), with at
the top level of the archive:

- `VERSION`;
- `app/`: the app Worker (`apps/worker-app`), `build/` and its static files, `assets/` (without `legal/`, an
  installation's);
- `engine/`: the raffle engine Worker (`apps/worker-engine`), `build/`;
- `live/`: the live Worker (`apps/worker-live`, the `RaffleRoom` Durable Objects), `build/`;
- `migrations/`: the D1 migrations (`migrations/sqlite`'s `.sql` files);
- `deploy.sh`: the deploy itself (`scripts/cloudflare-deploy.sh`; its header says what each subcommand does), so an
  installation deploys a release with that release's own logic, attested with it.

The combined Worker (`apps/worker`) is for local development only. Self-hosting builds from the tag.

Installations deploy a release with GitHub Actions, from their own repository, created from the template
repository, a copy of `deploy/cloudflare` ([below](#the-template-repository)): its
`VERSION` names the release (`VERSION.<environment>` instead, when the workflows' `CLOUDFLARE_ENV` names a
Wrangler environment and that file exists: several installations in one repository), and its two workflows,
`.github/workflows/deploy.yaml` and `restate-cleanup.yaml`, download that release's `worker.tar.gz` from this
repository, verify its attestation (signed by `release.yaml`, from a run on the tag `v<version>`) and run its
`deploy.sh`, one subcommand per step (`check-remote`, `migrate`, `deploy-live`, `upload-app` and
`deploy-engine`, `register-engine` and `promote-app`; `cleanup` daily). `deploy.sh` deploys the release it is part
of (its `VERSION`) and checks nothing of the installation's: not its version file, not the placeholders, not that
the three configurations agree. The workflows are thin and the
installation's own; nothing in `deploy.sh` is GitHub's: it reads the Workers' names from wrangler's dry run of
each configuration (for `CLOUDFLARE_ENV`), writes its outputs to `RAFI_OUTPUT_FILE`, its Markdown report to
`RAFI_REPORT_FILE` and the JSON it read and wrote to `RAFI_ARTIFACTS_DIR` (the workflows give it `$GITHUB_OUTPUT`,
`$GITHUB_STEP_SUMMARY` and a directory they upload), and installs nothing: it runs `npx -y wrangler` and `npx -y
@restatedev/restate`, which take the versions the installation's `package-lock.json` pins, installed with
`npm ci` (or `WRANGLER` and `RESTATE`), and reads JSON with `jq`. An installation is
its app Worker's name: Restate's deployment metadata names it as `instance`. So an installation created from
the template at a tag deploys that tag, and upgrading is changing `VERSION`.

## Once

In the repository's settings (**Settings > General > Releases**), enable **immutable releases**: a published
release's assets and its tag can't change after publishing, so `worker.tar.gz` under a version is the one GitHub
showed the digest of and attested, and the tag `v<version>` the template repository is updated from stays the
commit it was. Installations' deploys download the asset by its tag and verify its attestation.

Create the [template repository](#the-template-repository), `sagikazarmark/rafi-deploy-template`, public, and
mark it **Template repository** (**Settings > General**).

## Before the tag

Name the release in `deploy/cloudflare/VERSION` (`0.2.0`, or `0.2.0-rc.1` for a pre-release). The rehearsal refuses
a release that file doesn't name (an installation copied at the tag would deploy another release), and CI's
rehearsal and the Release workflow's manual run are of the version it names. The template's `INSTALL.md` and `README.md`
name the release in their commands too (the template it creates from, the release step 10 downloads, `/healthz`'s
answer, the engine Workers' names, the version files of Several installations): change them with `VERSION`
(`rg -n '0\.1\.0|0-1-0' deploy/cloudflare/*.md`); nothing checks those.

When a change to `scripts/cloudflare-deploy.sh` changes how it is driven (a subcommand, its environment or outputs,
what it expects of the installation's repository), change the template's workflows to fit, in the same commit, and
say in the release notes that operators copy them: nothing refuses an installation's older workflows, so a
subcommand they still call has to keep working for a release, or the notes say what breaks.

The release notes are generated from the merged pull requests; edit them on the release when an operator has to
act or should know:

- a migration, a new secret, a setting in `deploy/cloudflare`'s configurations or `.dev.vars.example` (operators
  carry those over by hand, into each `[env.<name>]` too, since environments don't inherit vars, bindings or required
  secrets);
- a change to the workflows: "copy `.github/workflows/deploy.yaml` and `restate-cleanup.yaml` from
  the template repository at this tag before changing `VERSION`" (for each installation, with its `CLOUDFLARE_ENV`), or
  a change to the workflows' secrets;
- a wrangler or Restate CLI `deploy.sh` now needs newer than an installation's `package-lock.json` may pin
  (operators run `npm update` and commit the lockfile), or a change to the template's `package.json`;
- whether the live Worker changed (its `live/build` differs from the previous release's): a deploy of a release
  that changes it drops every live connection, so say so;
- when a migration or the live channel would break the previous release's engine while it drains
  ([Upgrades](../operations/upgrades.md#what-a-deploy-does-to-a-raffle-in-flight)), or a change to the live
  Worker's requests would break the previous app's or engine's (a change to them takes two releases).

CI's `workers` job rehearses a release (of the version `deploy/cloudflare/VERSION` names) on every push.

## Rehearsing

Run the Release workflow by hand (**Actions > Release > Run workflow**): it runs CI, builds the release of the
version given (by default the one `deploy/cloudflare/VERSION` names), checks that the template's `VERSION` names it,
that its `package-lock.json` pins the devenv's wrangler (after a wrangler bump, `npm install
wrangler@<version> --package-lock-only` in `deploy/cloudflare`), and that its workflows download the release the
version file names (and call no `deploy.sh check`), and runs
its `deploy.sh dry-run` from a copy of `deploy/cloudflare` with the release unpacked into `release/` (the devenv's
wrangler, the Workers' names read from its dry runs), at the configurations' top level and as a Wrangler
environment (`[env.eu]` added to each configuration, with `RAFI_DO_JURISDICTION = "eu"`, `VERSION.eu`,
`CLOUDFLARE_ENV=eu`), publishing nothing. `dry_run` off publishes a tag that already exists, to finish a release whose run failed: run it
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

## The template repository

Operators create their repository from `sagikazarmark/rafi-deploy-template` (`deploy/cloudflare`'s INSTALL.md,
step 1), which mirrors `deploy/cloudflare`: its `main` is the latest release's, and each release's is tagged the
same, `v<version>`, which the release notes' "copy the workflows at this tag" means. Once the release is published,
update it, by hand for now: `deploy/cloudflare` at the tag becomes the repository's whole content.

```sh
gh repo clone sagikazarmark/rafi-deploy-template && cd rafi-deploy-template
git rm -rq --ignore-unmatch .
git -C <your Rafi checkout> archive v<version> deploy/cloudflare | tar -x --strip-components=2
git add -A && git commit -m "Rafi <version>"
git tag -s v<version> -m "Rafi <version>"
git push origin main v<version>
```

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
- Installations upgrade by naming it in their version file and pushing (copying the template's workflows at the tag
  first when the release notes say they changed); their deploy verifies the attestation and runs the release's own
  `deploy.sh`. A release's engine is the Worker `<engine name>-v<version, dashed>` (`rafi-engine-v0-2-0`;
  `rafi-engine-eu-v0-2-0` in a Wrangler environment `eu`), registered with each installation's Restate by its
  deploy (never over a newer release's), and once Restate reports the previous release's engine drained, the
  installation's Restate cleanup workflow removes its deployment from Restate, then deletes its Worker.
