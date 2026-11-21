import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createPochitaViteConfig } from '@pochita/config';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const kitRoot = process.env.POCHITA_KIT_ROOT
  ? path.resolve(process.env.POCHITA_KIT_ROOT)
  : path.resolve(__dirname, '../../../../pochita-webui-kit');

export default createPochitaViteConfig({
  outDir: path.resolve(__dirname, '../plugins/strata.manager/web'),
  port: 5178,
  kitRoot,
  surface: 'native-glass',
  server: {
    fs: {
      allow: [__dirname],
    },
  },
});
