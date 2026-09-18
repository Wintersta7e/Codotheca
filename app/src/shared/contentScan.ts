/**
 * §29.8's grant, as the settings surface states it.
 *
 * The language list is rendered rather than described because §29.2's rule 3 reads `BY_EXT`'s
 * `programming` flag, and that table was written for the language byte census: someone adding an
 * extension there to make a language appear in the language bar would otherwise silently widen
 * what this product reads off the user's disk. Rendering the list makes that a visible change to
 * a stated policy. `app/test/contentScanLanguages.test.ts` derives both sides and compares them.
 *
 * **No count is written here.** The checker derives both counts and fails at zero.
 */
export const CONTENT_SCAN_LANGUAGES = [
  'C',
  'C#',
  'C++',
  'Go',
  'Java',
  'JavaScript',
  'Kotlin',
  'Lua',
  'PHP',
  'Python',
  'Ruby',
  'Rust',
  'Shell',
  'Swift',
  'TypeScript',
] as const;

/** The control's label. It says what is read, not what is counted. */
export const CONTENT_SCAN_LABEL = 'Read the text of committed source files';

/** What is read, and from where. The basis is the commit, never the working copy. */
export const CONTENT_SCAN_NOTE =
  'OFF UNTIL YOU TURN IT ON · COMMITTED FILES ONLY, NEVER YOUR UNCOMMITTED EDITS · NOTHING IS UPLOADED';

/**
 * §29.8's consequence line. Turning the grant off deletes what it wrote, and a promise that
 * leaves the data behind is not the promise that was made.
 */
export const CONTENT_SCAN_CONSEQUENCE =
  'Turning this off deletes everything it read. The four answers above it — README, licence, ' +
  'tests, CI — come from file names and stay.';

/** The caption above the rendered language list. */
export const CONTENT_SCAN_LANGUAGES_CAPTION = 'Read in these languages:';
