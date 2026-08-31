/**
 * §8.6's palette sub-line opens with a `sigil`. The spec names the field and never defines the
 * mapping; §7.3a owns the only language list in the document, and these are the design's own
 * tags for it. A language outside the table has no sigil, and the caller drops the field.
 */
export const LANGUAGE_SIGILS: Readonly<Record<string, string>> = {
  rust: '.rs',
  typescript: '.ts',
  python: '.py',
  'c++': '.cpp',
  'c#': '.cs',
  javascript: '.js',
  java: '.java',
  go: '.go',
  shell: '.sh',
  lua: '.lua',
};

export function languageSigil(primaryLanguage: string | null): string | null {
  if (primaryLanguage === null) return null;
  const key = primaryLanguage.trim().toLowerCase();
  if (key === '') return null;
  return LANGUAGE_SIGILS[key] ?? null;
}
