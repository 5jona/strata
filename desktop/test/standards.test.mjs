import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DESKTOP_ROOT = path.resolve(__dirname, '..');

function walkFiles(dir, extensions, out = []) {
  if (!fs.existsSync(dir)) return out;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (
      entry.name.startsWith('.') ||
      entry.name === 'node_modules' ||
      entry.name === 'target' ||
      entry.name === 'dist' ||
      entry.name === 'gen'
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

test('all desktop TypeScript and Rust source files stay within the 300-line target', () => {
  const files = [
    ...walkFiles(path.join(DESKTOP_ROOT, 'src'), ['.ts', '.tsx', '.css']),
    ...walkFiles(path.join(DESKTOP_ROOT, 'crates'), ['.rs']),
    ...walkFiles(path.join(DESKTOP_ROOT, 'src-tauri/src'), ['.rs']),
  ];
  assert.ok(files.length >= 20, `Expected >= 20 source files, found ${files.length}`);

  const violations = [];
  for (const file of files) {
    const lines = fs.readFileSync(file, 'utf8').split('\n').length;
    if (lines > 300) {
      violations.push(`${path.relative(DESKTOP_ROOT, file)} (${lines} lines)`);
    }
  }
  assert.deepEqual(
    violations,
    [],
    `Desktop files exceeding 300-line limit:\n${violations.join('\n')}`,
  );
});

test('desktop package obeys architecture invariants (no sideveil-sdk, no forbidden globals)', () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(DESKTOP_ROOT, 'package.json'), 'utf8'));
  const allDeps = { ...pkg.dependencies, ...pkg.devDependencies };
  assert.equal('@gojona/sideveil-sdk' in allDeps, false, 'desktop must not depend on sideveil-sdk');
  assert.ok('@gojona/ui' in allDeps, 'desktop must reuse @gojona/ui');
  assert.ok('@gojona/config' in allDeps, 'desktop must reuse @gojona/config');

  const srcFiles = walkFiles(path.join(DESKTOP_ROOT, 'src'), ['.ts', '.tsx']);
  const forbidden = [
    { re: /window\.__POCHITA_HOST__/g, label: 'direct window.__POCHITA_HOST__' },
    { re: /window\.ksu\b/g, label: 'direct window.ksu' },
  ];
  for (const file of srcFiles) {
    const content = fs.readFileSync(file, 'utf8');
    for (const { re, label } of forbidden) {
      assert.equal(re.test(content), false, `${path.relative(DESKTOP_ROOT, file)} uses ${label}`);
    }
  }
});

test('compiled Vite dist bundle preserves Pochita theme bootstrap and lazy terminal chunk', () => {
  const distDir = path.join(DESKTOP_ROOT, 'dist');
  const htmlPath = path.join(distDir, 'index.html');
  assert.ok(fs.existsSync(htmlPath), `Missing built dist/index.html at ${htmlPath}`);

  const html = fs.readFileSync(htmlPath, 'utf8');
  assert.match(html, /<meta\s+name="color-scheme"\s+content="light dark"/i);
  assert.ok(
    html.includes('pochita:color-scheme'),
    'dist/index.html missing theme bootstrap script',
  );

  const assetsDir = path.join(distDir, 'assets');
  const jsFiles = fs.readdirSync(assetsDir).filter((f) => f.endsWith('.js'));
  assert.ok(
    jsFiles.length >= 2,
    `Expected lazy code-split terminal chunk (>= 2 JS assets), found ${jsFiles.length}`,
  );
});
