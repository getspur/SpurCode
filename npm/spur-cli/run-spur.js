#!/usr/bin/env node
"use strict";
// Thin bin shim: exec the downloaded native spur binary, forwarding argv,
// stdio (including the TTY for the TUI), and the exit code.
const { spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const { binaryPath, helperIsCompatible } = require("./platform");

const bin = binaryPath();
if (!fs.existsSync(bin) || !helperIsCompatible()) {
  // Installs with --ignore-scripts skip postinstall; fetch on first run.
  const installed = spawnSync(
    process.execPath,
    [path.join(__dirname, "install.js")],
    { stdio: "inherit" }
  );
  if (installed.status !== 0) {
    process.exit(installed.status === null ? 1 : installed.status);
  }
}

if (!helperIsCompatible()) {
  console.error("@getspur/spur-cli: installed embedding worker has incompatible version, protocol or platform");
  process.exit(1);
}

const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`@getspur/spur-cli: failed to run ${bin}: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status === null ? 1 : result.status);
