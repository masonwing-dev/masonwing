import { compile } from 'json-schema-to-typescript';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
const output = new URL('../packages/api-client/src/generated/', import.meta.url);
const check = process.argv.includes('--check');
let drift = 0;
if (!check) await mkdir(output, { recursive: true });
async function emit(name, content) {
  const path = new URL(name, output);
  if (!check) return writeFile(path, content);
  const current = await readFile(path, 'utf8').catch(() => null);
  if (current !== content) { console.error(`Generated type drift: ${name}`); drift += 1; }
}
const availableProducts = [];
for (const product of ['masonwing', 'gleanbird']) {
  const schemaUrl = new URL(`../contracts/${product}/contracts.json`, import.meta.url);
  const rawSchema = await readFile(schemaUrl, 'utf8').catch(() => null);
  if (!rawSchema) continue;
  availableProducts.push(product);
  const schema = JSON.parse(rawSchema);
  const aggregate = { ...schema, type: 'object', properties: Object.fromEntries(
    Object.keys(schema.$defs).map(name => [name, { $ref: `#/$defs/${name}` }])), additionalProperties: false };
  const text = await compile(aggregate, `${product}Contracts`, {
    bannerComment: '/* Generated from immutable spec by pnpm contracts:generate. Do not edit. */',
    unreachableDefinitions: true, additionalProperties: false,
  });
  await emit(`${product}.ts`, text);
}
const exportLines = availableProducts.map(p => `export type * as ${p.charAt(0).toUpperCase() + p.slice(1)} from './${p}';`).join('\n') + '\n';
await emit('contracts.ts', exportLines);
console.log(check ? `Checked contract types; ${drift} drifted files` : `Generated contract type namespaces for ${availableProducts.join(', ')}`);
process.exitCode = drift ? 1 : 0;
