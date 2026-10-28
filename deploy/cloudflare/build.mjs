#!/usr/bin/env node
// Puts Rafi in place for `wrangler deploy`: the release this installation pins
// (package.json, "rafi.version") into rafi/, checked against the release's
// SHA256SUMS, and this installation's legal documents (legal/) among the
// static files, where the Worker reads them.
//
//   node build.mjs            # fetch the release, then the documents
//   node build.mjs --local    # keep rafi/ as it is (a build from source:
//                             # scripts/cloudflare-artifact.sh in the Rafi
//                             # repository), only the documents
//
// RAFI_RELEASE_URL names where the release's files are, instead of the
// GitHub release (a mirror, or a test). The checksums travel with the
// release, so the check is against a corrupt or truncated download, not
// against whoever controls the download location. Node 18 or later; `tar`
// on the PATH.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const pkg = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));
const { version, repository } = pkg.rafi;
const dest = join(here, "rafi");
const local = process.argv.includes("--local");

const fail = (message) => {
  console.error(`build: ${message}`);
  process.exit(1);
};

if (!local) {
  const name = `rafi-cloudflare-${version}.tar.gz`;
  const base =
    process.env.RAFI_RELEASE_URL?.replace(/\/?$/, "/") ??
    `https://github.com/${repository}/releases/download/v${version}/`;
  console.log(`build: Rafi ${version} from ${base}${name}`);
  const [tarball, sums] = await Promise.all([download(base + name), download(base + "SHA256SUMS")]);
  const expected = new TextDecoder()
    .decode(sums)
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .find(([, file]) => file === name || file === `*${name}`)?.[0];
  if (!expected) fail(`SHA256SUMS of the release has no line for ${name}`);
  const actual = createHash("sha256").update(tarball).digest("hex");
  if (actual !== expected) fail(`${name} doesn't match SHA256SUMS: ${actual}, expected ${expected}`);

  // The tarball holds one directory, rafi/, which becomes rafi/ here.
  const tmp = mkdtempSync(join(tmpdir(), "rafi-"));
  const file = join(tmp, name);
  writeFileSync(file, tarball);
  rmSync(dest, { recursive: true, force: true });
  mkdirSync(dest);
  execFileSync("tar", ["-xzf", file, "-C", dest, "--strip-components=1"], { stdio: "inherit" });
  rmSync(tmp, { recursive: true, force: true });
}

if (!existsSync(join(dest, "build", "index.js"))) {
  fail(
    local
      ? "rafi/ has no Worker in it: build one with scripts/cloudflare-artifact.sh in the Rafi repository, into this directory's rafi/"
      : "the release has no Worker in it",
  );
}
// What the artifact says it is: a release's VERSION is its version; a build
// from source says what `git describe` said.
const shipped = existsSync(join(dest, "VERSION")) ? readFileSync(join(dest, "VERSION"), "utf8").trim() : "unknown";
if (!local && shipped !== version) fail(`the release's VERSION says ${shipped}, not ${version}`);

// The documents: the Worker refuses every request while they still carry
// the template marker, so say so now rather than after the deploy.
const legal = join(dest, "worker-assets", "legal");
mkdirSync(legal, { recursive: true });
let templates = false;
for (const doc of ["terms.md", "privacy.md"]) {
  const source = join(here, "legal", doc);
  if (!existsSync(source)) fail(`legal/${doc} is missing`);
  templates ||= readFileSync(source, "utf8").includes("**Template.**");
  copyFileSync(source, join(legal, doc));
}
console.log(`build: Rafi ${shipped} in rafi/, with legal/terms.md and legal/privacy.md`);
if (templates) {
  console.warn(
    "build: the legal documents are still the templates (they carry **Template.**): the Worker will refuse every request until reviewed documents replace them",
  );
}

/** The response body of `url`, or a failure. */
async function download(url) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) fail(`${url}: ${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}
