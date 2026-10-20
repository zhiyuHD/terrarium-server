import { createInterface } from 'node:readline';
import { migrateState } from './legacy-src/state.ts';
import { validAquarium } from './legacy-src/aquarium.ts';
for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
  try {
    const raw = JSON.parse(line);
    const state = raw?.kind === 'water' ? (validAquarium(raw) ? raw : undefined) : migrateState(raw);
    process.stdout.write(JSON.stringify(state ? { state } : { invalid: true }) + '\n');
  } catch { process.stdout.write('{"invalid":true}\n'); }
}
