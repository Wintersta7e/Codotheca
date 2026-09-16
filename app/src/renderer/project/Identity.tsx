/**
 * §8.5.1's identity block.
 *
 * [p2] §25.3 restores visibility, which §8.5.1 cut because *"visibility is a remote fact"* and
 * printing "public" under every local repository invents one. It is a remote fact still — so it
 * renders **only when it was observed**, and `PUBLIC` and `PRIVATE` **both render or neither
 * does**: rendering only `PRIVATE` makes its absence assert public, which is the invented fact
 * the field was cut for.
 *
 * The dot is §5.4a's, at the project-page size that section's table gives it, and its colours
 * come from the one derivation every surface reads. A project with no signal draws none —
 * absence is the honest render of *not computed*.
 */
import type { ReactElement } from 'react';

import type { ProjectRow, RemoteVisibility } from '../../generated/protocol';
import { conditionDot, DOT_SIZE_PX } from '../derive/condition';

/**
 * `<CONDITION> · <owner ·> <birth year> · <language> <· visibility>`. A segment with no value is
 * omitted rather than filled: an em dash here would be five ways of saying nothing in one line.
 */
export function identityLine(row: ProjectRow, visibility: RemoteVisibility | null): string | null {
  const parts: string[] = [];
  if (row.conditionSignal !== null) parts.push(row.conditionSignal.toUpperCase());
  // `owner` is NULL when it is the user, which is how the projection says "not worth naming".
  if (row.owner !== null) parts.push(row.owner.toUpperCase());
  if (row.birthYear !== null) parts.push(String(row.birthYear));
  if (row.primaryLanguage !== null) parts.push(row.primaryLanguage.toUpperCase());
  // Both words, or neither. The two branches are written out rather than uppercased from the
  // wire value so that a build capable of rendering one is structurally capable of the other —
  // AC-P2-25-8 reads this file for exactly that.
  if (visibility === 'public') parts.push('PUBLIC');
  else if (visibility === 'private') parts.push('PRIVATE');
  return parts.length === 0 ? null : parts.join(' · ');
}

export interface IdentityProps {
  row: ProjectRow;
  /**
   * §25.3's observed fact. `null` is *unobserved* and omits the segment entirely, exactly as
   * `owner` already does — never a placeholder, and never one of the two words.
   */
  visibility: RemoteVisibility | null;
}

export function Identity({ row, visibility }: IdentityProps): ReactElement {
  const dot = conditionDot({
    signal: row.conditionSignal,
    isReference: row.isReference,
    isArchived: row.isArchived,
  });
  const line = identityLine(row, visibility);
  const size = DOT_SIZE_PX.projectPage;

  return (
    <div>
      {line === null ? null : (
        <div className="cp-identity-line">
          {dot === null ? null : (
            // §11.7 would otherwise announce the condition twice: the word is printed beside the
            // mark, so this one carries no name of its own. The hero's dot stands alone and does.
            <div
              className="cp-dot"
              data-testid="cp-identity-dot"
              aria-hidden="true"
              style={{
                width: `${String(size)}px`,
                height: `${String(size)}px`,
                ...(dot.fill === null ? {} : { background: dot.fill }),
                ...(dot.ring === null ? {} : { border: dot.ring }),
                ...(dot.glow === null ? {} : { boxShadow: dot.glow }),
              }}
            />
          )}
          <span>{line}</span>
        </div>
      )}
      <h1 className="cp-name">{row.name}</h1>
      {row.description === null ? null : (
        <p className="cp-description" data-testid="cp-description">
          {row.description}
        </p>
      )}
    </div>
  );
}
