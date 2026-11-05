"use strict";
// Maps the npm process platform/arch onto a spur release asset name, as
// published by `cargo xtask dist` to getspur/spur-releases. macOS artifacts
// are architecture-specific so each install downloads only its native Mach-O.
// Skills ship as a platform-independent spur-skills-<ver>.tar.gz.
const path = require("path");

function assetFor(version, platform = process.platform, arch = process.arch) {
  if (platform === "darwin" && arch === "arm64") {
    return { asset: `spur-${version}-aarch64-apple-darwin`, exe: false };
  }
  if (platform === "darwin" && arch === "x64") {
    return { asset: `spur-${version}-x86_64-apple-darwin`, exe: false };
  }
  if (platform === "linux" && arch === "arm64") {
    return { asset: `spur-${version}-aarch64-unknown-linux-gnu`, exe: false };
  }
  if (platform === "linux" && arch === "x64") {
    return { asset: `spur-${version}-x86_64-unknown-linux-gnu`, exe: false };
  }
  if (platform === "win32" && arch === "x64") {
    return { asset: `spur-${version}-x86_64-pc-windows-msvc.exe`, exe: true };
  }
  throw new Error(
    `@getspur/spur-cli: no prebuilt spur binary for ${platform}/${arch}. ` +
      "See https://github.com/getspur/spur-releases/releases for available artifacts."
  );
}

function skillsAssetFor(version) {
  return `spur-skills-${version}.tar.gz`;
}

function binaryPath(exe = process.platform === "win32") {
  return path.join(__dirname, "bin", exe ? "spur.exe" : "spur");
}

function helperPath(exe = process.platform === "win32") {
  return path.join(__dirname, "bin", `spur-embedding-worker${exe ? ".exe" : ""}`);
}

function helperIsCompatible(executable = helperPath()) {
  // Reuse the supervised helper's startup allowance. spawnSync waits for reaping;
  // force termination so a metadata executable ignoring SIGTERM cannot hang npm.
  const result = require("child_process").spawnSync(executable, ["--compatibility"], {
    encoding: "utf8",
    timeout: 120000,
    killSignal: "SIGKILL",
  });
  if (result.error || result.status !== 0 || result.stderr) return false;
  try {
    const metadata = JSON.parse(result.stdout);
    return metadata.version === require("./package.json").version &&
      metadata.protocol === 2 &&
      metadata.os === ({ darwin: "macos", win32: "windows" }[process.platform] || process.platform) &&
      metadata.arch === ({ arm64: "aarch64", x64: "x86_64" }[process.arch] || process.arch);
  } catch (_) {
    return false;
  }
}

module.exports = { assetFor, binaryPath, helperPath, helperIsCompatible, skillsAssetFor };
