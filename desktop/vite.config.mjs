import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createPochitaViteConfig } from '@gojona/config';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const generatedDir = path.resolve(__dirname, '../webui/src/generated');
const kitRoot = process.env.POCHITA_KIT_ROOT
  ? path.resolve(process.env.POCHITA_KIT_ROOT)
  : undefined;

export default createPochitaViteConfig({
  outDir: path.resolve(__dirname, 'dist'),
  port: 5179,
  ...(kitRoot ? { kitRoot } : {}),
  surface: 'standalone',
  resolve: {
    alias: {
      '@strata/generated': generatedDir,
    },
  },
  server: {
    strictPort: true,
    fs: {
      allow: [__dirname, generatedDir],
    },
  },
});
