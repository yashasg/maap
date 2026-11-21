import { createHash } from 'node:crypto';
import { readFile, rename, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

// Fontshare permits self-hosting but not redistribution in the public repo or container image.
const file = fileURLToPath(new URL('../public/fonts/satoshi-variable.woff2', import.meta.url));
const source =
  'https://cdn.fontshare.com/wf/NWBQYJIM7GCZ5XWD7D26ARB3VDY55ZRT/K63EV2KZIGKLE7RANQ2U42S6SVHU5RJ7/X6XYTKIVDUW7GZTZPZNN4EUM5KH54KHF.woff2';
const valid = bytes =>
  createHash('sha256').update(bytes).digest('hex') ===
  'e739aff9b4d02c264341d6d4872edcda28e79373aeda936f659566a1cd3eb47f';

async function ensureFont() {
  try {
    if (valid(await readFile(file))) return;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  const response = await fetch(source, { signal: AbortSignal.timeout(10000) });
  if (!response.ok) throw new Error(`Fontshare returned HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (!valid(bytes)) throw new Error('Unexpected Satoshi font checksum');
  const temp = `${file}.${process.pid}.tmp`;
  await writeFile(temp, bytes);
  await rename(temp, file);
}

if (!process.env.DISABLE_UI) {
  await ensureFont().catch(error =>
    console.warn(`Satoshi unavailable; using system font: ${error.message}`),
  );
}
