import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { STRATA_MANIFEST } from '../src/manifest.ts';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const WEBUI_ROOT = path.resolve(__dirname, '..');
const REPO_ROOT = path.resolve(WEBUI_ROOT, '..');

function walkFiles(dir, extensions, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (
      entry.name.startsWith('.') ||
      entry.name === 'node_modules' ||
      entry.name === 'target' ||
      entry.name === 'out' ||
      entry.name === 'dist' ||
      entry.name === 'assets' ||
      entry.name === 'generated'
    ) {
      continue;
    }
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walkFiles(full, extensions, out);
    } else if (extensions.some((ext) => entry.name.endsWith(ext))) {
      out.push(full);
    }
  }
  return out;
}

test('webui source files obey bridge, token, and refresh-rate standards', () => {
  const files = walkFiles(path.join(WEBUI_ROOT, 'src'), ['.ts', '.tsx', '.css']);
  assert.ok(files.length > 0, 'Expected strata/webui/src source files to exist');

  const forbiddenPatterns = [
    { re: /window\.__POCHITA_HOST__/g, label: 'direct window.__POCHITA_HOST__ access' },
    { re: /window\.ksu\b/g, label: 'direct window.ksu access' },
    { re: /window\.postMessage\s*\(/g, label: 'raw window.postMessage call' },
    {
      re: /\b(?:fps|measuredFps|refreshRate)\s*=\s*Math\.min\s*\(\s*\d+\s*,|Math\.min\s*\(\s*(?:60|90|120)\s*,/gi,
      label: 'hardcoded FPS ceiling',
    },
  ];

  for (const file of files) {
    const rel = path.relative(REPO_ROOT, file);
    const content = fs.readFileSync(file, 'utf8');
    for (const { re, label } of forbiddenPatterns) {
      assert.equal(re.test(content), false, `${rel} contains forbidden ${label}`);
    }
  }
});

test('compiled SideVeil plugin package matches manifest.ts and Vite + Pochita bundle invariants', () => {
  const manifestPath = path.join(REPO_ROOT, 'module/plugins/strata.manager/manifest.json');
  const manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));

  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.apiVersion, 1);
  assert.equal('runtimeMode' in manifest, false, 'v1 manifest must not contain legacy runtimeMode');
  assert.equal(manifest.id, STRATA_MANIFEST.id);
  assert.equal(manifest.name, STRATA_MANIFEST.name);
  assert.equal(manifest.entry, STRATA_MANIFEST.entry);
  assert.equal(manifest.backend, STRATA_MANIFEST.backend);
  assert.equal(manifest.surface?.backdrop, STRATA_MANIFEST.surface?.backdrop);
  assert.deepEqual(manifest.capabilities, STRATA_MANIFEST.capabilities);

  const webDir = path.join(REPO_ROOT, 'out/stage/plugins/strata.manager/web');
  const htmlPath = path.join(webDir, 'index.html');
  const iconPath = path.join(webDir, 'icon.svg');
  assert.ok(fs.existsSync(htmlPath), `Missing compiled bundle: ${htmlPath}`);
  assert.ok(fs.existsSync(iconPath), `Missing compiled icon: ${iconPath}`);

  for (const targetHtml of [htmlPath, path.join(REPO_ROOT, 'out/stage/webroot/index.html')]) {
    assert.ok(fs.existsSync(targetHtml), `Missing compiled bundle: ${targetHtml}`);
    const html = fs.readFileSync(targetHtml, 'utf8');
    assert.match(html, /<meta\s+name="color-scheme"\s+content="light dark"/i);
    assert.ok(
      html.includes('pochita:color-scheme'),
      `${targetHtml} missing theme bootstrap script`,
    );
    assert.match(
      html,
      /<script\s+type="module"\s+crossorigin\s+src="\.\/assets\/[^"]+\.js"><\/script>/,
      `${targetHtml} must reference compiled Vite module JS asset`,
    );
    assert.match(
      html,
      /<link\s+rel="stylesheet"\s+crossorigin\s+href="\.\/assets\/[^"]+\.css">/,
      `${targetHtml} must reference compiled Vite CSS asset`,
    );

    const inlineScripts = [...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/gi)].filter(
      (match) => !/\bsrc=/i.test(match[0]) && match[1].trim().length > 0,
    );
    assert.equal(inlineScripts.length, 1, `${targetHtml} must only contain theme bootstrap script`);

    const assetsDir = path.join(path.dirname(targetHtml), 'assets');
    for (const assetName of fs.readdirSync(assetsDir)) {
      const content = fs.readFileSync(path.join(assetsDir, assetName), 'utf8');
      assert.equal(
        /\bxterm\b/i.test(content),
        false,
        `${path.relative(REPO_ROOT, path.join(assetsDir, assetName))} must stay ultra-lean with zero xterm.js`,
      );
    }
  }
});

test('all managed source and test files stay within the 400-line hard limit', () => {
  const scanSpecs = [
    { dir: path.join(WEBUI_ROOT, 'src'), extensions: ['.ts', '.tsx', '.css'] },
    { dir: path.join(REPO_ROOT, 'daemon/crates'), extensions: ['.rs'] },
    { dir: path.join(REPO_ROOT, 'tools/strata-debug-agent/src'), extensions: ['.rs'] },
    { dir: path.join(REPO_ROOT, 'tools/strata-mcp/src'), extensions: ['.rs'] },
    { dir: path.join(REPO_ROOT, 'scripts'), extensions: ['.py', '.sh', '.mjs'] },
    { dir: path.join(REPO_ROOT, 'module'), extensions: ['.sh'] },
  ];
  const explicitFiles = [path.join(REPO_ROOT, 'module/container/host-exec')].filter((file) =>
    fs.existsSync(file),
  );

  const files = [...explicitFiles];
  for (const { dir, extensions } of scanSpecs) {
    files.push(...walkFiles(dir, extensions));
  }

  const violations = [];
  for (const file of files) {
    const lines = fs.readFileSync(file, 'utf8').split('\n').length;
    if (lines > 400) {
      violations.push(`${path.relative(REPO_ROOT, file)} (${lines} lines)`);
    }
  }

  assert.deepEqual(
    violations,
    [],
    `Source files exceeding 400-line hard limit:\n${violations.join('\n')}`,
  );
});

test('SideVeil StrataManagerApp remains a zero-tab zero-IME quick panel and webui handwritten files stay <= 300 lines', () => {
  const pluginApp = fs.readFileSync(path.join(WEBUI_ROOT, 'src/StrataManagerApp.tsx'), 'utf8');
  assert.equal(/<Tabs\b/.test(pluginApp), false, 'StrataManagerApp must not render Tabs');
  assert.equal(/<Input\b/.test(pluginApp), false, 'StrataManagerApp must not render Input fields');
  assert.equal(
    /backupStart|CONTAINER_BACKUP_START/.test(pluginApp),
    false,
    'StrataManagerApp must not trigger backups',
  );

  const tabsApp = fs.readFileSync(path.join(WEBUI_ROOT, 'src/ksu/KsuTabsWorkspace.tsx'), 'utf8');
  assert.match(tabsApp, /value="overview"/);
  assert.match(tabsApp, /value="services"/);
  assert.match(tabsApp, /value="config"/);
  assert.match(tabsApp, /value="diagnostics"/);

  const pluginAppLines = pluginApp.split('\n').length;
  assert.ok(
    pluginAppLines <= 220,
    `StrataManagerApp.tsx must be <= 220 lines (got ${pluginAppLines})`,
  );
  const ksuApp = fs.readFileSync(path.join(WEBUI_ROOT, 'src/ksu/KsuStrataApp.tsx'), 'utf8');
  const ksuAppLines = ksuApp.split('\n').length;
  assert.ok(ksuAppLines <= 180, `KsuStrataApp.tsx must be <= 180 lines (got ${ksuAppLines})`);

  const webuiFiles = walkFiles(path.join(WEBUI_ROOT, 'src'), ['.ts', '.tsx', '.css']);
  const oversized = [];
  for (const file of webuiFiles) {
    const lines = fs.readFileSync(file, 'utf8').split('\n').length;
    if (lines > 300) {
      oversized.push(`${path.relative(REPO_ROOT, file)} (${lines} lines)`);
    }
    const content = fs.readFileSync(file, 'utf8');
    assert.equal(
      /\/data\/local\/tmp/.test(content),
      false,
      `${path.relative(REPO_ROOT, file)} must not hardcode /data/local/tmp`,
    );
  }
  assert.deepEqual(
    oversized,
    [],
    `webui/src handwritten files exceeding 300-line limit:\n${oversized.join('\n')}`,
  );
});
