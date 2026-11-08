// Collects the license texts of every crate compiled into the Windows binary
// into THIRD_PARTY_LICENSES.txt (required when distributing MIT/Apache/BSD code).
// Usage: node scripts/third-party-licenses.mjs
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';

const meta = JSON.parse(execFileSync('cargo', [
  'metadata', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc',
], { maxBuffer: 256 * 1024 * 1024 }).toString());

const byId = new Map(meta.packages.map((p) => [p.id, p]));
const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
const root = meta.resolve.root;

// Walk normal (runtime) dependencies only: build/dev deps are not shipped.
const seen = new Set();
const stack = [root];
while (stack.length) {
  const id = stack.pop();
  if (seen.has(id)) continue;
  seen.add(id);
  for (const dep of nodes.get(id)?.deps ?? []) {
    if (dep.dep_kinds.some((k) => k.kind === null)) stack.push(dep.pkg);
  }
}
seen.delete(root);

const LICENSE_FILE = /^(licen[sc]e|copying|notice|copyright)([-_.].*)?$/i;
const sections = [];
for (const id of [...seen].sort((a, b) => byId.get(a).name.localeCompare(byId.get(b).name))) {
  const pkg = byId.get(id);
  const dir = dirname(pkg.manifest_path);
  let files = [];
  try { files = readdirSync(dir).filter((f) => LICENSE_FILE.test(f)).sort(); } catch { /* none */ }
  const texts = files.map((f) => `--- ${f} ---\n${readFileSync(join(dir, f), 'utf8').trim()}`);
  sections.push([
    '='.repeat(78),
    `${pkg.name} ${pkg.version}`,
    `License: ${pkg.license ?? pkg.license_file ?? 'see files'}`,
    pkg.repository ? `Source: ${pkg.repository}` : null,
    '',
    texts.length ? texts.join('\n\n') : '(No license file shipped in the crate; see the license identifier above.)',
  ].filter((l) => l !== null).join('\n'));
}

const header = [
  'Rustcord includes the following third-party software.',
  'Each component is provided under its own license, reproduced below.',
  `Components: ${sections.length}`,
  '',
].join('\n');
writeFileSync('THIRD_PARTY_LICENSES.txt', header + sections.join('\n\n') + '\n');
console.log(`THIRD_PARTY_LICENSES.txt: ${sections.length} components`);
