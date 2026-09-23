import type { ReactElement } from 'react';

import { PART_SEPARATOR } from './checkForms';

/**
 * `PART_SEPARATOR` as an element. Without a stylesheet a row still reads as parts; with one, the
 * row lays its parts out apart and hides this (`.cp-sep` in `projectPage.css`), so a styled row
 * does not carry a dot between a name and its badge.
 */
export function PartSeparator(): ReactElement {
  return <span className="cp-sep">{PART_SEPARATOR}</span>;
}
