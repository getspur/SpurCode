"use strict";
// Downloads the platform binary for this package's version from
// getspur/spur-releases, verifies it against the release SHA256SUMS, and
// installs it as ./bin/spur[.exe]. Also downloads spur-skills-<ver>.tar.gz and
// extracts share/spur/skills next to this package so SkillCatalog package
// discovery finds skills (bin/../share/spur/skills).
//
// Runs at postinstall; run-spur.js also invokes it lazily when the binary is
// missing (--ignore-scripts installs).
const fs = require("fs");
const path = require("path");
const crypto = require("crypto");
const { execFileSync } = require("child_process");
const { assetFor, binaryPath, helperPath, helperIsCompatible, skillsAssetFor } = require("./platform");

const RELEASES = "https://github.com/getspur/spur-releases/releases/download";

async function fetchBytes(url) {
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) {
    throw new Error(`download failed (HTTP ${res.status}) for ${url}`);
  }
  return Buffer.from(await res.arrayBuffer());
}

function expectedSha256(sumsText, asset) {
  for (const line of sumsText.split("\n")) {
    const fields = line.trim().split(/\s+/);
    if (fields.length === 2 && fields[1] === asset) {
      return fields[0].toLowerCase();
    }
  }
  return null;
}

function verifyChecksum(sumsText, asset, bytes) {
  const expected = expectedSha256(sumsText, asset);
  if (!expected) {
    throw new Error(`SHA256SUMS has no entry for ${asset}`);
  }
  const actual = crypto.createHash("sha256").update(bytes).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `checksum mismatch for ${asset}: expected ${expected}, got ${actual}`
    );
  }
}

function extractSkillsArchive(tarPath, destDir) {
  // System tar is available on macOS, Linux, and modern Windows 10+.
  execFileSync("tar", ["-xzf", tarPath, "-C", destDir], { stdio: "inherit" });
}

async function main() {
  const version = require("./package.json").version;
  const { asset, exe } = assetFor(version);
  const skillsAsset = skillsAssetFor(version);
  const base = `${RELEASES}/v${version}`;

  const sums = (await fetchBytes(`${base}/SHA256SUMS`)).toString("utf8");

  const bytes = await fetchBytes(`${base}/${asset}`);
  verifyChecksum(sums, asset, bytes);
  const helperAsset = asset.replace(/^spur-/, "spur-embedding-worker-");
  const helperBytes = await fetchBytes(`${base}/${helperAsset}`);
  verifyChecksum(sums, helperAsset, helperBytes);

  // Skills are not embedded in the binary. Extract the release skills
  // tarball so package_asset_candidates finds <pkg>/share/spur/skills.
  const skillsBytes = await fetchBytes(`${base}/${skillsAsset}`);
  verifyChecksum(sums, skillsAsset, skillsBytes);

  // Finish downloads, checksums and extraction before replacing any installed
  // component. A failed helper fetch must leave the previous pair usable.
  const stage = fs.mkdtempSync(path.join(__dirname, ".spur-install-"));
  const skillsTar = path.join(stage, skillsAsset);
  fs.writeFileSync(skillsTar, skillsBytes);
  try {
    extractSkillsArchive(skillsTar, stage);
    const stagedSkills = path.join(stage, "share", "spur", "skills");
    if (!fs.statSync(stagedSkills).isDirectory()) {
      throw new Error("skills archive layout must be share/spur/skills/");
    }
    const dest = binaryPath(exe);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    const stagedHost = path.join(stage, path.basename(dest));
    const stagedHelper = path.join(stage, path.basename(helperPath(exe)));
    fs.writeFileSync(stagedHost, bytes, { mode: 0o755 });
    fs.writeFileSync(stagedHelper, helperBytes, { mode: 0o755 });
    if (!helperIsCompatible(stagedHelper)) {
      throw new Error("incompatible embedding worker version, protocol or platform");
    }
    fs.renameSync(stagedHelper, helperPath(exe));
    fs.renameSync(stagedHost, dest);
    const shareRoot = path.join(__dirname, "share");
    fs.rmSync(shareRoot, { recursive: true, force: true });
    fs.renameSync(path.join(stage, "share"), shareRoot);
    console.log(`spur ${version} and embedding worker installed: ${dest}`);
  } finally {
    fs.rmSync(stage, { recursive: true, force: true });
  }

  const skillProbe = path.join(__dirname, "share", "spur", "skills");
  if (!fs.existsSync(skillProbe)) {
    throw new Error(
      `skills extract missing ${skillProbe}; archive layout must be share/spur/skills/`
    );
  }
  console.log(`spur skills installed: ${skillProbe}`);
}

main().catch((err) => {
  console.error(
    `@getspur/spur-cli install failed: ${err && err.message ? err.message : err}`
  );
  process.exit(1);
});
