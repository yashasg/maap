import assert from 'node:assert/strict';
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { mock, test } from 'node:test';
import { pathToFileURL } from 'node:url';

test('a bad upstream response never overwrites the local font', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'maap-font-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'scripts'));
  await mkdir(join(dir, 'public/fonts'), { recursive: true });
  const script = join(dir, 'scripts/fetch-font.mjs');
  const font = join(dir, 'public/fonts/satoshi-variable.woff2');
  await copyFile(new URL('../../scripts/fetch-font.mjs', import.meta.url), script);
  await writeFile(font, 'existing file');
  t.mock.method(globalThis, 'fetch', () => Promise.resolve(new Response('not a font')));
  const warn = t.mock.method(console, 'warn', mock.fn());
  await import(pathToFileURL(script).href);
  assert.match(warn.mock.calls[0].arguments[0], /checksum/);
  assert.equal(await readFile(font, 'utf8'), 'existing file');
});
