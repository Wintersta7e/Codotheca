import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { ConditionSignal } from '../../generated/protocol';
import { DOT_SIZE_PX } from '../derive/condition';
import { ConditionDotMark } from './ConditionDotMark';

afterEach(cleanup);

const draw = (
  signal: ConditionSignal | null,
  over: { isReference?: boolean; isArchived?: boolean; surface?: 'card' | 'hero' } = {},
): void => {
  render(
    <ConditionDotMark
      signal={signal}
      isReference={over.isReference ?? false}
      isArchived={over.isArchived ?? false}
      surface={over.surface ?? 'card'}
    />,
  );
};

describe('the dot', () => {
  it('renders at the size §5.4a gives the surface, on both surfaces', () => {
    draw('dormant');
    expect(screen.getByTestId('cdt-dot').style.width).toBe(`${String(DOT_SIZE_PX.gridTile)}px`);
    cleanup();
    draw('dormant', { surface: 'hero' });
    expect(screen.getByTestId('cdt-dot').style.width).toBe(`${String(DOT_SIZE_PX.heroTile)}px`);
  });

  it('sits inside band 1, at the inset the band table gives it', () => {
    draw('dormant');
    const dot = screen.getByTestId('cdt-dot');
    expect(dot.style.right).toBe('10px');
    expect(dot.style.top).toBe('10px');
    expect(dot.style.borderRadius).toBe('50%');
  });

  it('takes the hero inset on the hero, which is a different band table', () => {
    draw('dormant', { surface: 'hero' });
    const dot = screen.getByTestId('cdt-dot');
    expect(dot.style.right).toBe('11px');
    expect(dot.style.top).toBe('11px');
  });

  it('carries the class plan 12 motion tiers already select', () => {
    draw('dormant');
    expect(screen.getByTestId('cdt-dot').className).toContain('cdt-dot');
  });

  it('leaves the hover scale to the stylesheet, so the tier can drop it', () => {
    draw('live');
    expect(screen.getByTestId('cdt-dot').style.transform).toBe('');
  });
});

describe('the accessible name is the word, never the colour', () => {
  it('names the enum member', () => {
    draw('neglected');
    expect(screen.getByText('Condition: neglected')).toBeTruthy();
  });

  it('hides the disc itself from the tree, so the name is the only thing announced', () => {
    draw('neglected');
    expect(screen.getByTestId('cdt-dot').getAttribute('aria-hidden')).toBe('true');
  });
});

describe('a project with no signal draws nothing at all', () => {
  it('renders no node and no name', () => {
    const { container } = render(
      <ConditionDotMark signal={null} isReference={false} isArchived={false} surface="card" />,
    );
    expect(container.innerHTML).toBe('');
    expect(screen.queryByText(/^Condition: /)).toBeNull();
  });
});

describe('the overrides are applied before the ladder, by plan 09', () => {
  it('takes the reference ring and the archived fill from that one table', () => {
    draw('idle', { isReference: true });
    const reference = screen.getByTestId('cdt-dot');
    expect(reference.style.border).not.toBe('');
    cleanup();
    draw('idle', { isArchived: true });
    expect(screen.getByTestId('cdt-dot').style.background).not.toBe('');
  });

  it('still names the band, because the override changes the paint and not the word', () => {
    draw('idle', { isReference: true });
    expect(screen.getByText('Condition: idle')).toBeTruthy();
  });
});
