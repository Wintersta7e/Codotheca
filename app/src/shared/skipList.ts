/**
 * §4.3's skip list as the user reads it.
 *
 * Shown verbatim on the roots screen and in settings, captioned as the privacy policy. Order is
 * §4.3's — caches, then build outputs, then system paths — and is never alphabetised, so it can
 * be checked against a real machine. `app/test/skipList.test.ts` pins every string to the core's
 * own list, because a privacy policy that misspells what it matches is false.
 */
export const EXCLUSION_LIST: readonly string[] = [
  'node_modules',
  '.venv',
  'venv',
  'target',
  'vendor',
  '.cargo/registry',
  'go/pkg/mod',
  '.terraform/modules',
  '.local/share/nvim/lazy',
  '.vim/bundle',
  '.oh-my-zsh',
  'Pods',
  '.m2',
  '.gradle',
  '.pyenv',
  '.rustup',
  '.nvm',
  'dist',
  'build',
  '.next',
  '__pycache__',
  '$RECYCLE.BIN',
  'System Volume Information',
  '/nix/store',
  '/var/lib/docker',
  '/proc',
  '/sys',
  '/snap',
  'AppData',
  // §24.3b: this app's own staging directory, where a partial clone lives until it is renamed
  // into place. It is in the rendered policy because the policy states what is skipped, and
  // this is skipped — a privacy policy that omits an entry is as false as one that misspells it.
  '.codotheca-installing',
];

/** §10.1b's caption for the list. */
export const EXCLUSION_CAPTION = 'This list is the privacy policy. Editable in settings.';

/** How many chips fit before the rest go behind the expander (§10.1b: three rows fit). */
export const EXCLUSION_CHIPS_BEFORE_EXPANDER = 11;
