/**
 * §11.3a's notification contract, in **one** place both the drawer and the shell read.
 *
 * The drawer states the three that exist; the shell posts the one phase 3 may fire. Two copies of
 * one sentence drift, and §11.7 forbids a developer's invention on the one surface a user cannot
 * dismiss before reading — so the sentence has one owner and both sides read it.
 *
 * It lives under `shared/` rather than in the settings module because the shell cannot import a
 * `.tsx` file: the node tsconfig sets no `--jsx`, and putting the string behind a component would
 * make the shell restate it.
 */
export const NOTIFICATION_LINES = [
  { label: 'A one-line summary on Sunday', note: 'SILENT ON AN EMPTY WEEK' },
  {
    label: 'A critical advisory in a project you have installed',
    // [p3] §32 lands the read this line needed, so it is the first of the three that may fire.
    note: 'NEEDS DEPENDENCY ADVISORIES',
  },
  { label: 'Your wrap is ready', note: 'NEEDS A WRAP' },
] as const;

/** §11.3a's second line — the one §32.12 posts. Quoted, never restated. */
export const ADVISORY_NOTICE_TITLE = NOTIFICATION_LINES[1].label;
