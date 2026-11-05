"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");
const test = require("node:test");

test("postinstall installs the verified worker beside the host in a path with spaces", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "spur install layout "));
  try {
    const pkg = path.join(root, "package with spaces");
    const release = path.join(root, "release");
    fs.mkdirSync(pkg);
    fs.mkdirSync(release);
    for (const file of ["install.js", "platform.js", "package.json"]) {
      fs.copyFileSync(path.join(__dirname, file), path.join(pkg, file));
    }
    const version = require("./package.json").version;
    const { asset, exe } = require("./platform").assetFor(version);
    const helperAsset = asset.replace(/^spur-/, "spur-embedding-worker-");
    const skillsAsset = `spur-skills-${version}.tar.gz`;
    const skillRoot = path.join(root, "skills", "share", "spur", "skills");
    fs.mkdirSync(skillRoot, { recursive: true });
    fs.writeFileSync(path.join(skillRoot, "marker"), "skills");
    execFileSync("tar", ["-czf", path.join(release, skillsAsset), "-C", path.join(root, "skills"), "share"]);
    fs.writeFileSync(path.join(release, asset), "host fixture");
    const metadata = { version, protocol: 2, os: process.platform === "darwin" ? "macos" : process.platform, arch: process.arch === "arm64" ? "aarch64" : "x86_64" };
    const worker = `#!/bin/sh\nprintf '%s' '${JSON.stringify(metadata)}'\n`;
    fs.writeFileSync(path.join(release, helperAsset), worker);
    const sums = [asset, helperAsset, skillsAsset].map((name) => {
      const hash = crypto.createHash("sha256").update(fs.readFileSync(path.join(release, name))).digest("hex");
      return `${hash}  ${name}`;
    }).join("\n");
    fs.writeFileSync(path.join(release, "SHA256SUMS"), sums);
    const preload = path.join(root, "fetch-fixture.js");
    fs.writeFileSync(preload, `const fs = require('node:fs');\nconst path = require('node:path');\nglobal.fetch = async url => { const p = path.join(process.env.SPUR_TEST_RELEASE, path.basename(new URL(url).pathname)); try { const data = fs.readFileSync(p); return { ok: true, arrayBuffer: async () => data }; } catch (_) { return { ok: false, status: 404 }; } };\n`);
    const result = spawnSync(process.execPath, ["--require", preload, path.join(pkg, "install.js")], {
      encoding: "utf8",
      env: { ...process.env, SPUR_TEST_RELEASE: release },
    });
    assert.equal(result.status, 0, result.stderr);
    const suffix = exe ? ".exe" : "";
    assert.equal(fs.readFileSync(path.join(pkg, "bin", `spur${suffix}`), "utf8"), "host fixture");
    assert.equal(fs.readFileSync(path.join(pkg, "bin", `spur-embedding-worker${suffix}`), "utf8"), worker);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "spur pair ü "));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const pkg = path.join(root, "package with spaces");
  const release = path.join(root, "release");
  fs.mkdirSync(pkg);
  fs.mkdirSync(release);
  for (const file of ["install.js", "platform.js", "run-spur.js", "package.json"]) {
    fs.copyFileSync(path.join(__dirname, file), path.join(pkg, file));
  }
  const version = require("./package.json").version;
  const { asset } = require("./platform").assetFor(version);
  const helper = asset.replace(/^spur-/, "spur-embedding-worker-");
  const skills = `spur-skills-${version}.tar.gz`;
  const skillRoot = path.join(root, "skills", "share", "spur", "skills");
  fs.mkdirSync(skillRoot, { recursive: true });
  fs.writeFileSync(path.join(skillRoot, "marker"), "skills");
  execFileSync("tar", ["-czf", path.join(release, skills), "-C", path.join(root, "skills"), "share"]);
  fs.writeFileSync(path.join(release, asset), "#!/bin/sh\nprintf 'host ran'\n");
  const metadata = { version, protocol: 2, os: process.platform === "darwin" ? "macos" : process.platform, arch: process.arch === "arm64" ? "aarch64" : "x86_64" };
  const worker = `#!/bin/sh\nprintf '%s' '${JSON.stringify(metadata)}'\n`;
  fs.writeFileSync(path.join(release, helper), worker);
  fs.mkdirSync(path.join(pkg, "bin"));
  const hostPath = path.join(pkg, "bin", "spur");
  const helperPath = path.join(pkg, "bin", "spur-embedding-worker");
  fs.writeFileSync(hostPath, "#!/bin/sh\nprintf 'old host'\n", { mode: 0o755 });
  fs.writeFileSync(helperPath, worker, { mode: 0o755 });
  const preload = path.join(root, "fetch.js");
  fs.writeFileSync(preload, `const fs = require('node:fs'); const path = require('node:path'); global.fetch = async url => { try { const data = fs.readFileSync(path.join(process.env.SPUR_TEST_RELEASE, path.basename(new URL(url).pathname))); return { ok: true, arrayBuffer: async () => data }; } catch (_) { return { ok: false, status: 404 }; } };`);
  function sums() {
    fs.writeFileSync(path.join(release, "SHA256SUMS"), [asset, helper, skills].map(name => `${crypto.createHash("sha256").update(fs.readFileSync(path.join(release, name))).digest("hex")}  ${name}`).join("\n"));
  }
  sums();
  return { pkg, release, asset, helper, worker, hostPath, helperPath, sums,
    run(file) { return spawnSync(process.execPath, [path.join(pkg, file)], { encoding: "utf8", env: { ...process.env, SPUR_TEST_RELEASE: release, NODE_OPTIONS: `--require="${preload}"` } }); } };
}

for (const failure of ["missing", "checksum"]) {
  test(`postinstall rejects ${failure} helper before replacing the installed pair`, t => {
    const f = fixture(t);
    const original = fs.readFileSync(f.hostPath, "utf8");
    if (failure === "missing") fs.rmSync(path.join(f.release, f.helper));
    else fs.appendFileSync(path.join(f.release, f.helper), "corrupt");
    const result = f.run("install.js");
    assert.notEqual(result.status, 0, "host-only upgrade must fail");
    assert.match(result.stderr, failure === "missing" ? /404/ : /checksum mismatch/);
    assert.equal(fs.readFileSync(f.hostPath, "utf8"), original);
    assert.equal(fs.readFileSync(f.helperPath, "utf8"), f.worker);
  });
}

for (const failure of ["missing", "mismatch"]) {
  test(`lazy launch repairs ${failure} helper before running host`, t => {
    const f = fixture(t);
    if (failure === "missing") fs.rmSync(f.helperPath);
    else fs.writeFileSync(f.helperPath, "#!/bin/sh\nprintf '{\"version\":\"old\"}'\n");
    const result = f.run("run-spur.js");
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /host ran/);
    assert.equal(fs.readFileSync(f.helperPath, "utf8"), f.worker);
  });
}

for (const [field, value] of [["version", "old"], ["protocol", 0], ["os", "wrong"], ["arch", "wrong"]]) {
  test(`postinstall rejects incompatible worker ${field} before replacing host`, t => {
    const f = fixture(t);
    const original = fs.readFileSync(f.hostPath, "utf8");
    const metadata = { version: require("./package.json").version, protocol: 2, os: process.platform === "darwin" ? "macos" : process.platform, arch: process.arch === "arm64" ? "aarch64" : "x86_64", [field]: value };
    fs.writeFileSync(path.join(f.release, f.helper), `#!/bin/sh\nprintf '%s' '${JSON.stringify(metadata)}'\n`);
    f.sums();
    const result = f.run("install.js");
    assert.notEqual(result.status, 0, "mismatched companion must prevent publication");
    assert.match(result.stderr, /incompatible/);
    assert.equal(fs.readFileSync(f.hostPath, "utf8"), original);
    assert.equal(fs.readFileSync(f.helperPath, "utf8"), f.worker);
  });
}
