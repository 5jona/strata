import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createPochitaViteConfig } from '@gojona/config';
import { mergeConfig } from 'vite';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const kitRoot = process.env.POCHITA_KIT_ROOT
  ? path.resolve(process.env.POCHITA_KIT_ROOT)
  : undefined;

const baseConfig = createPochitaViteConfig({
  outDir: path.resolve(__dirname, '../out/stage/webroot'),
  port: 5179,
  ...(kitRoot ? { kitRoot } : {}),
  surface: 'standalone',
  server: {
    fs: {
      allow: [__dirname],
    },
  },
});

export default mergeConfig(baseConfig, {
  root: path.resolve(__dirname, 'ksu'),
});
