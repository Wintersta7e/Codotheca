/**
 * **AC-P3-29-19** — the three promise sites agree.
 *
 * §10.1's paragraph, §10.1b's row 2 and §11.3's group-4 row are read **as rendered**, each string
 * is asserted non-empty **before** anything is asserted about it, and none may claim that source
 * files are never read while `contentScanEnabled` can be true.
 *
 * The non-empty check first is not ceremony: a suite once passed nine of ten assertions
 * vacuously against an empty string and reported success.
 */
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { CONSENT_PARAGRAPH, CONSENT_ROWS } from '../firstrun/copy.js';
import {
  CONTENT_SCAN_CONSEQUENCE,
  CONTENT_SCAN_LABEL,
  CONTENT_SCAN_NOTE,
} from '../../shared/contentScan.js';
import { ScanGroups, type ScanGroupsProps } from './groupsScan.js';

afterEach(cleanup);

const props: ScanGroupsProps = {
  roots: null,
  targets: null,
  onSetEnabled: vi.fn(),
  onSetDescend: vi.fn(),
  onAddFolder: vi.fn(),
  onRescan: vi.fn(),
  contentScanEnabled: false,
  onSetContentScan: vi.fn(),
  slots: {},
};

/** A claim that the product never reads source files. Any of these is now false for a user who
 *  said yes, which is the whole reason the three sites move together. */
const NEVER_READ =
  /source files are never read|never reads? (the contents|the text) of your source|does not read the (contents|text) of your source files(?![^.]*\bunless\b)/i;

it('AC-P3-29-19 every promise site is a real, non-empty rendered string', () => {
  const { container } = render(<ScanGroups {...props} />);
  const groupFour = container.querySelector('[data-group="scanning"]')?.textContent ?? '';
  const scanned: readonly { readonly site: string; readonly text: string }[] = [
    { site: '§10.1 paragraph', text: CONSENT_PARAGRAPH },
    { site: '§10.1b row 2 body', text: CONSENT_ROWS[1]?.body ?? '' },
    { site: '§10.1b row 2 note', text: CONSENT_ROWS[1]?.note ?? '' },
    { site: '§11.3 group 4, as rendered', text: groupFour },
  ];
  // `console.warn` rather than stderr: this file is in the web project and has no node types.
  console.warn(`contentScanPromise: promise sites scanned ${String(scanned.length)}`);
  expect(scanned.length).toBeGreaterThan(0);
  for (const { site, text } of scanned) {
    expect(text, `${site} is empty, so every assertion about it would pass vacuously`).not.toBe('');
    expect(text.length, site).toBeGreaterThan(20);
    expect(text, site).not.toMatch(NEVER_READ);
  }
});

it('AC-P3-29-19 the first-run row names the setting it has become, still a statement', () => {
  const row = CONSENT_ROWS[1];
  expect(row).toBeTruthy();
  expect(row?.body ?? '').not.toBe('');
  // §29.8: the ask is in context, so §10.1b's *a checkbox there would store a preference nothing
  // reads* expires without this row becoming a control on that screen.
  expect(row?.kind).toBe('statement');
  expect(row?.note ?? '').toMatch(/OFF UNTIL YOU TURN IT ON/);
  expect(row?.note ?? '').not.toMatch(/NOT BUILT YET/);
});

it('AC-P3-29-19 the group-4 row carries the grant, with its consequence line', () => {
  render(<ScanGroups {...props} />);
  expect(CONTENT_SCAN_LABEL).not.toBe('');
  expect(CONTENT_SCAN_NOTE).not.toBe('');
  expect(CONTENT_SCAN_CONSEQUENCE).not.toBe('');
  expect(screen.getByRole('switch', { name: CONTENT_SCAN_LABEL })).toBeTruthy();
  // The consequence is the half a user acts on: turning it off deletes what it read.
  expect(CONTENT_SCAN_CONSEQUENCE).toMatch(/deletes everything it read/);
  expect(screen.getByText(CONTENT_SCAN_CONSEQUENCE)).toBeTruthy();
});

it('AC-P3-29-19 the paragraph says the grant exists and says it is off', () => {
  expect(CONSENT_PARAGRAPH).not.toBe('');
  expect(CONSENT_PARAGRAPH).toMatch(/off until you do/);
  expect(CONSENT_PARAGRAPH).toMatch(/Nothing is uploaded/);
});
