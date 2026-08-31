import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const repoFile = (relative: string): string =>
  readFileSync(new URL(`../../${relative}`, import.meta.url), 'utf8');

describe('the projection carries the art seed', () => {
  it('declares both fields on ProjectRow in the schema', () => {
    const schema: unknown = JSON.parse(repoFile('protocol/schema/protocol.json'));
    const types = (schema as { types: Record<string, { fields: Record<string, string> }> }).types;
    const row = types['ProjectRow'];
    expect(row?.fields['seedBasename']).toBe('String');
    expect(row?.fields['rerollOffset']).toBe('u32');
  });

  it('spells them exactly as ProjectDetail already does, so nothing is renamed on the way in', () => {
    const schema: unknown = JSON.parse(repoFile('protocol/schema/protocol.json'));
    const types = (schema as { types: Record<string, { fields: Record<string, string> }> }).types;
    expect(types['ProjectDetail']?.fields['seedBasename']).toBe(
      types['ProjectRow']?.fields['seedBasename'],
    );
    expect(types['ProjectDetail']?.fields['rerollOffset']).toBe(
      types['ProjectRow']?.fields['rerollOffset'],
    );
  });

  it('generates them into the TypeScript projection', () => {
    const generated = repoFile('app/src/generated/protocol.ts');
    const block = /export interface ProjectRow \{([\s\S]*?)\n\}/.exec(generated)?.[1] ?? '';
    expect(block).toMatch(/readonly seedBasename: string;/);
    expect(block).toMatch(/readonly rerollOffset: number;/);
  });

  it('generates them into the Rust projection too, or one side draws a card the other cannot', () => {
    const generated = repoFile('core/src/protocol.rs');
    const block = /pub struct ProjectRow \{([\s\S]*?)\n\}/.exec(generated)?.[1] ?? '';
    expect(block).toMatch(/pub seed_basename: String,/);
    expect(block).toMatch(/pub reroll_offset: u32,/);
  });
});
