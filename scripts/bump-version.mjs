#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SEMVER_RE =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const read = (rel) => fs.readFileSync(path.join(ROOT, rel), 'utf8');
const write = (rel, text) => fs.writeFileSync(path.join(ROOT, rel), text, 'utf8');

function fail(msg) {
  console.error(`[bump-version] Error: ${msg}`);
  process.exit(1);
}

function parseSemver(raw, label) {
  const m = raw.match(SEMVER_RE);
  if (!m) fail(`${label} "${raw}" is not valid SemVer`);
  const [major, minor, patch] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (minor >= 100 || patch >= 100) {
    fail(`${label} "${raw}" requires minor < 100 and patch < 100 for versionCode`);
  }
  return { version: raw, versionCode: major * 10000 + minor * 100 + patch };
}

function parseArgs(argv) {
  let check = false;
  let tag = null;
  let version = null;
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === '--check') check = true;
    else if (arg === '--tag') tag = argv[++i] ?? fail('--tag requires a value');
    else if (arg.startsWith('--tag=')) tag = arg.slice(6);
    else if (arg === '-h' || arg === '--help') {
      console.log('Usage: node scripts/bump-version.mjs [<X.Y.Z>] [--check] [--tag vX.Y.Z]');
      process.exit(0);
    } else if (arg.startsWith('-')) fail(`Unknown option: ${arg}`);
    else if (!version) version = arg;
    else fail(`Unexpected argument: ${arg}`);
  }
  if (!check && !version && !tag) fail('Specify <X.Y.Z> or --check');
  if ((check || tag) && version) fail('Cannot combine <X.Y.Z> with --check or --tag');
  return { check: check || Boolean(tag), tag, version };
}

function replaceOnce(rel, regex, replacement, label) {
  const src = read(rel);
  if (!regex.test(src)) fail(`Missing ${label} pattern in ${rel}`);
  write(rel, src.replace(regex, replacement));
}

function matchGroup(rel, regex, label) {
  const m = read(rel).match(regex);
  if (!m) fail(`Missing ${label} in ${rel}`);
  return m[1];
}

function topChangelogVersion() {
  for (const line of read('docs/CHANGELOG.md').split(/\r?\n/)) {
    const m = line.match(/^## \[([^\]]+)\](?:\s+-\s+\d{4}-\d{2}-\d{2})?$/);
    if (m) return m[1];
  }
  return null;
}

const JSON_TARGETS = [
  'webui/package.json',
  'module/plugins/strata.manager/manifest.json',
  'desktop/package.json',
  'desktop/src-tauri/tauri.conf.json',
];

const WORKSPACE_TOML_RE = /\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m;
const PACKAGE_TOML_RE = /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m;

function runCheck(tag) {
  const ver = matchGroup('module/module.prop', /^version=v(.+)$/m, 'version');
  const code = Number(matchGroup('module/module.prop', /^versionCode=(\d+)$/m, 'versionCode'));
  const parsed = parseSemver(ver, 'module/module.prop version');
  if (code !== parsed.versionCode) {
    fail(`module/module.prop versionCode ${code} != expected ${parsed.versionCode}`);
  }
  const checks = [
    ['daemon/Cargo.toml', matchGroup('daemon/Cargo.toml', WORKSPACE_TOML_RE, 'workspace.package.version')],
    ['tools/strata-debug-agent/Cargo.toml', matchGroup('tools/strata-debug-agent/Cargo.toml', PACKAGE_TOML_RE, 'package.version')],
    ['tools/strata-mcp/Cargo.toml', matchGroup('tools/strata-mcp/Cargo.toml', PACKAGE_TOML_RE, 'package.version')],
    ['desktop/Cargo.toml', matchGroup('desktop/Cargo.toml', WORKSPACE_TOML_RE, 'workspace.package.version')],
    ['webui/src/manifest.ts', matchGroup('webui/src/manifest.ts', /version:\s*'([^']+)'/, 'manifest version')],
    ...JSON_TARGETS.map((rel) => [rel, JSON.parse(read(rel)).version]),
    ['docs/CHANGELOG.md', topChangelogVersion()],
  ];
  for (const [name, actual] of checks) {
    if (actual !== ver) fail(`${name} version "${actual}" != "${ver}"`);
  }
  if (tag !== null) {
    if (!tag.startsWith('v')) fail(`Tag "${tag}" must start with "v"`);
    if (tag.slice(1) !== ver) fail(`Tag "${tag}" does not match repository version "${ver}"`);
  }
  console.log(`[bump-version] OK: all targets and docs/CHANGELOG.md match v${ver} (versionCode=${code})`);
}

function runBump(rawVersion) {
  const { version: v, versionCode: code } = parseSemver(rawVersion, 'Target version');
  replaceOnce('module/module.prop', /^version=v.+$/m, `version=v${v}`, 'version');
  replaceOnce('module/module.prop', /^versionCode=\d+$/m, `versionCode=${code}`, 'versionCode');
  replaceOnce('daemon/Cargo.toml', /(\[workspace\.package\][\s\S]*?^version\s*=\s*)"[^"]+"/m, `$1"${v}"`, 'workspace.package.version');
  replaceOnce('desktop/Cargo.toml', /(\[workspace\.package\][\s\S]*?^version\s*=\s*)"[^"]+"/m, `$1"${v}"`, 'workspace.package.version');
  replaceOnce('tools/strata-debug-agent/Cargo.toml', /(\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m, `$1"${v}"`, 'package.version');
  replaceOnce('tools/strata-mcp/Cargo.toml', /(\[package\][\s\S]*?^version\s*=\s*)"[^"]+"/m, `$1"${v}"`, 'package.version');
  replaceOnce('webui/src/manifest.ts', /(version:\s*)'[^']+'/, `$1'${v}'`, 'manifest version');
  for (const rel of JSON_TARGETS) {
    const json = JSON.parse(read(rel));
    json.version = v;
    write(rel, `${JSON.stringify(json, null, 2)}\n`);
  }
  if (topChangelogVersion() !== v) {
    const raw = read('docs/CHANGELOG.md');
    const heading = `## [${v}] - ${new Date().toISOString().slice(0, 10)}\n\n### Changed\n\n- Release v${v}.\n\n`;
    const idx = raw.search(/^## \[/m);
    write('docs/CHANGELOG.md', idx === -1 ? `${raw.trimEnd()}\n\n${heading}` : `${raw.slice(0, idx)}${heading}${raw.slice(idx)}`);
  }
  console.log(`[bump-version] Updated all targets and docs/CHANGELOG.md to v${v} (versionCode=${code})`);
}

const { check, tag, version } = parseArgs(process.argv.slice(2));
if (check) runCheck(tag);
else runBump(version);
