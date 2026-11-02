/**
 * Host half of dsh-mirsa: registers the bundled `mirsa-unsafe-check` skill.
 *
 * The skill lives inside this package, so the registration resolves its own
 * directory instead of relying on a machine-specific skills path.
 */
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const name = 'dsh-mirsa';
export const inject = ['skills'];

const PROVIDER = 'dsh-mirsa';
const RANK = 600;
const SKILLS = ['mirsa-unsafe-check'];

function parseSkill(path) {
  const raw = readFileSync(path, 'utf8');
  const frontmatter = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/u.exec(raw);
  if (frontmatter === null) throw new Error(`${path} has no YAML frontmatter`);
  const description = /^description:\s*(.+)$/mu.exec(frontmatter[1]);
  if (description === null) throw new Error(`${path} has no description`);
  return {
    description: description[1].trim(),
    content: raw.slice(frontmatter[0].length).trim(),
  };
}

export function apply(ctx) {
  const root = fileURLToPath(new URL('./skills/', import.meta.url));
  const candidates = SKILLS.map((skill) => {
    const directory = join(root, skill);
    const locator = join(directory, 'SKILL.md');
    const { description } = parseSkill(locator);
    return {
      name: skill,
      description,
      invocation: { modelInvocable: true, userInvocable: true },
      provider: PROVIDER,
      source: 'bundled',
      rank: RANK,
      resourceBase: { kind: 'directory', path: directory.split('\\').join('/') },
      locator,
    };
  });
  const provider = {
    name: PROVIDER,
    list: () => Promise.resolve(candidates),
    async get(candidate) {
      const { rank: _rank, locator, ...summary } = candidate;
      return { ...summary, content: parseSkill(locator).content };
    },
  };
  ctx.skills.registerProvider(() => provider);
}
