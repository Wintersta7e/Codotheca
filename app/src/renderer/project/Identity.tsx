/**
 * §8.5.1's identity block. `public|private` is cut: visibility is a remote fact, and printing
 * "public" under every local repository invents one.
 *
 * The dot is §5.4a's, at the project-page size that section's table gives it, and its colours
 * come from the one derivation every surface reads. A project with no signal draws none —
 * absence is the honest render of *not computed*.
 */
import type { ReactElement } from 'react';

import type { ProjectRow } from '../../generated/protocol';
import { conditionDot, DOT_SIZE_PX } from '../derive/condition';

/**
 * `<CONDITION> · <owner ·> <birth year> · <language>`. A segment with no value is omitted rather
 * than filled: an em dash here would be four ways of saying nothing in one line.
 */
export function identityLine(row: ProjectRow): string | null {
  const parts: string[] = [];
  if (row.conditionSignal !== null) parts.push(row.conditionSignal.toUpperCase());
  // `owner` is NULL when it is the user, which is how the projection says "not worth naming".
  if (row.owner !== null) parts.push(row.owner.toUpperCase());
  if (row.birthYear !== null) parts.push(String(row.birthYear));
  if (row.primaryLanguage !== null) parts.push(row.primaryLanguage.toUpperCase());
  return parts.length === 0 ? null : parts.join(' · ');
}

export interface IdentityProps {
  row: ProjectRow;
}

export function Identity({ row }: IdentityProps): ReactElement {
  const dot = conditionDot({
    signal: row.conditionSignal,
    isReference: row.isReference,
    isArchived: row.isArchived,
  });
  const line = identityLine(row);
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
