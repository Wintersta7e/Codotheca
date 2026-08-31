import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AttentionChip, DEFAULT_CHIP_ACCENT, type AttentionChipProps } from './AttentionChip.js';

afterEach(cleanup);

const base: AttentionChipProps = {
  count: 4,
  label: 'UNPUSHED',
  subLine: 'WORK ONLY ON THIS DISK',
  active: false,
  onActivate: vi.fn(),
};

const draw = (over: Partial<AttentionChipProps> = {}): ReturnType<typeof render> =>
  render(<AttentionChip {...base} {...over} />);

describe('AttentionChip', () => {
  it('renders §8.0b’s number, label and sub-line and reports its pressed state', () => {
    draw();
    const button = screen.getByRole('button', { name: /UNPUSHED/ });
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(document.querySelector('.cdt-attention-count')?.textContent).toBe('4');
    expect(document.querySelector('.cdt-attention-label')?.textContent).toBe('UNPUSHED');
    expect(document.querySelector('.cdt-attention-sub')?.textContent).toBe(
      'WORK ONLY ON THIS DISK',
    );
  });

  // Never render unknown as zero. A collection whose query no longer parses counted nothing,
  // and there is no digit, dash or dimmed figure that says that honestly.
  it('renders no number node at all when the count is null', () => {
    const { container } = draw({ count: null });
    expect(container.querySelector('[data-part="count"]')).toBeNull();
    expect(container.querySelector('.cdt-attention-count')).toBeNull();
    expect(container.textContent).not.toMatch(/\d/);
    expect(container.textContent).not.toContain('—');
  });

  it('renders a zero it was actually given', () => {
    const { container } = draw({ count: 0 });
    expect(container.querySelector('[data-part="count"]')?.textContent).toBe('0');
  });

  // §8.3a's soft errors are struck inside the sub-line, which a string could not carry.
  it('takes a node for the sub-line, not only a string', () => {
    draw({ subLine: <span className="probe">lang:rust</span> });
    expect(document.querySelector('.cdt-attention-sub .probe')?.textContent).toBe('lang:rust');
  });

  it('defaults the accent to the neutral and lets a caller set it', () => {
    const { container } = draw();
    expect(
      container
        .querySelector<HTMLElement>('.cdt-attention-chip')
        ?.style.getPropertyValue('--cdt-chip-accent'),
    ).toBe(DEFAULT_CHIP_ACCENT);
    cleanup();
    const set = draw({ accent: 'var(--sig)' });
    expect(
      set.container
        .querySelector<HTMLElement>('.cdt-attention-chip')
        ?.style.getPropertyValue('--cdt-chip-accent'),
    ).toBe('var(--sig)');
  });

  it('takes an accessible name when the spec states one, and none when it does not', () => {
    draw({ accessibleName: 'Rust work, 2 projects' });
    expect(screen.getByRole('button', { name: 'Rust work, 2 projects' })).toBeTruthy();
    cleanup();
    draw();
    expect(screen.getByRole('button', { name: /UNPUSHED/ }).getAttribute('aria-label')).toBeNull();
  });

  it('cannot be activated while disabled', () => {
    const onActivate = vi.fn();
    draw({ disabled: true, onActivate });
    const button = screen.getByRole('button', { name: /UNPUSHED/ });
    expect((button as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(button);
    expect(onActivate).not.toHaveBeenCalled();
  });
});
