import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DENSITY_TILE_PX } from './viewState.js';
import { DENSITY_LABELS, DensityControl, densityControlName } from './DensityControl.js';

afterEach(cleanup);

describe('DENSITY_LABELS', () => {
  it("renders 12b's third step as §8.0a's word", () => {
    // 12b names the step `roomy`; the visible word and the accessible name are `LARGE` and
    // `Density: large`. The translation is here, and it is the only place it happens.
    expect(DENSITY_LABELS.compact).toBe('COMPACT');
    expect(DENSITY_LABELS.default).toBe('DEFAULT');
    expect(DENSITY_LABELS.roomy).toBe('LARGE');
  });
  it('labels every step the ladder can reach, so no cycle lands on undefined', () => {
    for (const px of DENSITY_TILE_PX) expect(densityControlName(px)).toMatch(/^Density: \w+$/);
  });
});

describe('densityControlName', () => {
  it('carries both visible words, so voice control can say what it sees', () => {
    expect(densityControlName(148)).toBe('Density: compact');
    expect(densityControlName(186)).toBe('Density: default');
    expect(densityControlName(232)).toBe('Density: large');
  });
  it('names a step for a stored value off the ladder rather than the raw number', () => {
    expect(densityControlName(240)).toBe('Density: large');
  });
});

describe('DensityControl', () => {
  it('is a real button — the prototype holds zero', () => {
    render(<DensityControl density={186} viewMode="grid" showKey onCycle={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Density: default' })).toBeTruthy();
  });

  it('shows the word and never a pixel count', () => {
    render(<DensityControl density={186} viewMode="grid" showKey onCycle={vi.fn()} />);
    expect(screen.getByRole('button').textContent).toBe('DENSITYDEFAULT');
    expect(screen.getByRole('button').textContent).not.toMatch(/\d/);
  });

  it('announces the changed value on an element that keeps focus', () => {
    const { container } = render(
      <DensityControl density={186} viewMode="grid" showKey onCycle={vi.fn()} />,
    );
    expect(container.querySelector('[aria-live="polite"]')?.textContent).toBe('DEFAULT');
  });

  it('cycles three steps and wraps', () => {
    const onCycle = vi.fn();
    const { rerender } = render(
      <DensityControl density={148} viewMode="grid" showKey onCycle={onCycle} />,
    );
    fireEvent.click(screen.getByRole('button'));
    expect(onCycle).toHaveBeenLastCalledWith(186);
    rerender(<DensityControl density={186} viewMode="grid" showKey onCycle={onCycle} />);
    fireEvent.click(screen.getByRole('button'));
    expect(onCycle).toHaveBeenLastCalledWith(232);
    rerender(<DensityControl density={232} viewMode="grid" showKey onCycle={onCycle} />);
    fireEvent.click(screen.getByRole('button'));
    expect(onCycle).toHaveBeenLastCalledWith(148);
  });

  it('sheds the key and keeps the value', () => {
    render(<DensityControl density={148} viewMode="grid" showKey={false} onCycle={vi.fn()} />);
    expect(screen.getByRole('button').textContent).toBe('COMPACT');
    expect(screen.getByRole('button').getAttribute('aria-label')).toBe('Density: compact');
  });

  it('is not rendered at all in list', () => {
    // §11.3a's dead-switch rule: density cannot act on the audit table.
    const { container } = render(
      <DensityControl density={186} viewMode="list" showKey onCycle={vi.fn()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('paints the control label at the decision floor', () => {
    const { container } = render(
      <DensityControl density={186} viewMode="grid" showKey onCycle={vi.fn()} />,
    );
    const key = container.querySelector('.cdt-shelf-control-key') as HTMLElement;
    expect(key.className).toContain('cdt-shelf-control-key');
    expect(container.innerHTML).not.toMatch(/#7a8896|#6c7885|#4a5560/);
  });

  it('names no destructive operation', () => {
    const { container } = render(
      <DensityControl density={186} viewMode="grid" showKey onCycle={vi.fn()} />,
    );
    expect(container.innerHTML).not.toMatch(/FORGET/i);
  });
});
