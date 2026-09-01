import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { RootsScreen, moreChipLabel, visibleExclusions } from './RootsScreen';
import { refusedRow, toRow } from './rootRows';
import * as copy from './copy';
import { EXCLUSION_CAPTION, EXCLUSION_LIST } from '../../shared/skipList';
import type { RootAdd, RootSuggestion } from '../../generated/protocol';

afterEach(cleanup);

const suggestion = (over: Partial<RootSuggestion> = {}): RootSuggestion => ({
  pathDisplay: '/somewhere/dev',
  kind: 'linux',
  distro: '',
  provenance: 'gitconfig',
  provenanceDetail: 'includeif',
  hits: 3,
  preTicked: true,
  ...over,
});

function draw(over: Partial<Parameters<typeof RootsScreen>[0]> = {}): {
  onToggleRoot: ReturnType<typeof vi.fn>;
  onConsent: ReturnType<typeof vi.fn>;
  onAddFolder: ReturnType<typeof vi.fn>;
  onConfirmLarge: ReturnType<typeof vi.fn>;
  onDig: ReturnType<typeof vi.fn>;
} {
  const deps = {
    onToggleRoot: vi.fn(),
    onConsent: vi.fn(),
    onAddFolder: vi.fn(),
    onConfirmLarge: vi.fn(),
    onDig: vi.fn(),
  };
  const rows = [
    toRow(suggestion(), true),
    toRow(
      suggestion({
        pathDisplay: '/somewhere/synced',
        provenance: 'cloud_synced',
        hits: null,
        preTicked: false,
      }),
      false,
    ),
  ];
  render(
    <RootsScreen
      deps={deps}
      rows={rows}
      ticked={new Set(['/somewhere/dev'])}
      consented
      pendingConfirm={null}
      tier="full"
      busy={false}
      {...over}
    />,
  );
  return deps;
}

// Criterion 12: first run asks zero configuration questions. Every control on this screen is a
// confirmation or an escape — there is no field, no select and no radio group.
test('the screen asks nothing', () => {
  draw();
  expect(screen.queryByRole('textbox')).toBeNull();
  expect(screen.queryByRole('combobox')).toBeNull();
  expect(screen.queryByRole('radio')).toBeNull();
  expect(screen.queryByRole('spinbutton')).toBeNull();
});

// §10.1a: this sentence does more trust work than any privacy paragraph.
test('the list says where it came from, in the caption and on every row', () => {
  draw();
  expect(screen.getByText(copy.ROOTS_LIST_CAPTION)).toBeTruthy();
  expect(screen.getByText('GITCONFIG · includeIf gitdir:/somewhere/dev/')).toBeTruthy();
  expect(screen.getByText('CLOUD-SYNCED · SCANNING MAY TRIGGER DOWNLOADS')).toBeTruthy();
});

// §10.1b: the count is provenance hits, not repositories, and never a fabricated zero.
test('a row with no source named it reads the unknown glyph and never a zero', () => {
  draw();
  const synced = screen.getByText('/somewhere/synced').closest('[data-testid="fr-root-row"]');
  expect(synced).not.toBeNull();
  expect(within(synced as HTMLElement).getByText(copy.UNKNOWN_GLYPH)).toBeTruthy();
  expect(within(synced as HTMLElement).queryByText('0')).toBeNull();
  expect(within(synced as HTMLElement).getByText('HITS')).toBeTruthy();
});

// §10.1a: cloud-sync roots are shown unchecked. The reason is true and self-interested rather
// than moralising, and it prevents silently costing someone gigabytes of metered bandwidth.
test('a cloud-synced row arrives unticked and says why', () => {
  draw();
  const boxes = screen.getAllByRole('checkbox');
  expect(boxes[0]?.getAttribute('aria-checked')).toBe('true');
  expect(boxes[1]?.getAttribute('aria-checked')).toBe('false');
});

test('ticking a row asks the gate, and never mutates a row itself', () => {
  const deps = draw();
  fireEvent.click(screen.getAllByRole('checkbox')[1]!);
  expect(deps.onToggleRoot).toHaveBeenCalledWith('/somewhere/synced');
});

// §10.1b: three rows, one of them a control. A checkbox on rows 2 and 3 would store a preference
// nothing reads.
test('exactly one consent row is a control and the other two are statements', () => {
  draw();
  const consent = screen.getByTestId('fr-consent');
  expect(within(consent).getAllByRole('checkbox')).toHaveLength(1);
  expect(within(consent).getAllByTestId('fr-consent-dot')).toHaveLength(2);
  for (const dot of within(consent).getAllByTestId('fr-consent-dot')) {
    expect(dot.closest('button')).toBeNull();
  }
});

// §10.1: this is the one claim the screen invites the user to check against their own disk.
test('WHAT EXACTLY holds the consent paragraph, verbatim', () => {
  draw();
  expect(screen.queryByText(copy.CONSENT_PARAGRAPH)).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: copy.WHAT_EXACTLY_LABEL }));
  expect(screen.getByText(copy.CONSENT_PARAGRAPH)).toBeTruthy();
});

// §10.1b: three rows of chips fit and the remainder go behind a `+ n MORE` chip expanding in
// place, which is what §10.1's "expandable" means.
test('the exclusion list is drawn in full behind one chip, in the core list order', () => {
  draw();
  expect(visibleExclusions(false)).toHaveLength(11);
  expect(moreChipLabel(EXCLUSION_LIST.length - 11)).toBe('+ 18 MORE');
  fireEvent.click(screen.getByRole('button', { name: '+ 18 MORE' }));
  for (const entry of EXCLUSION_LIST) expect(screen.getByText(entry)).toBeTruthy();
  // A privacy policy that misspells what it matches is false.
  expect(screen.getByText('$RECYCLE.BIN')).toBeTruthy();
  expect(screen.getByText(EXCLUSION_CAPTION)).toBeTruthy();
});

// §10.1b: unticking is honoured, and there is no product behind that tick.
test('withholding consent leaves DIG inert with the reason beside it', () => {
  const deps = draw({ consented: false });
  const dig = screen.getByRole('button', { name: copy.DIG_LABEL });
  expect(dig.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(dig);
  expect(deps.onDig).not.toHaveBeenCalled();
  expect(screen.getByText(copy.DIG_INERT_NOTE)).toBeTruthy();
  expect(screen.queryByText(copy.DIG_NOTE)).toBeNull();
});

// §10.1b: the prototype's "You can use the app while it works." is false by the design's own
// performance rule — every full-screen flow unmounts the shelf.
test('with consent DIG commits and claims only the true thing', () => {
  const deps = draw();
  expect(screen.getByText(copy.DIG_NOTE)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: copy.DIG_LABEL }));
  expect(deps.onDig).toHaveBeenCalledTimes(1);
});

test('DIG is inert while a commit is in flight, so one press cannot start two scans', () => {
  const deps = draw({ busy: true });
  fireEvent.click(screen.getByRole('button', { name: copy.DIG_LABEL }));
  expect(deps.onDig).not.toHaveBeenCalled();
});

// §17 and the Global Constraints: phase 1 has no destructive operation at all.
test('no rendered string on this screen carries a destructive verb', () => {
  draw();
  expect(document.body.textContent).not.toMatch(/\bFORGET\b|\bdelete\b|\buninstall\b/i);
});

const refusal = (because: RootAdd['refusedBecause'], dirs: number | null): RootAdd => ({
  root: null,
  refusedBecause: because,
  estimatedDirs: dirs,
});

// §10.1b: three are absolute and are listed with no tick slot at all, path at the ornament grey,
// reason in the provenance slot.
test('an absolute refusal is a row with no control on it', () => {
  draw({
    rows: [refusedRow(refusal('home_without_narrowing', null), '/somewhere')],
    ticked: new Set<string>(),
  });
  const row = screen.getByTestId('fr-root-row');
  expect(row.tagName).not.toBe('BUTTON');
  expect(within(row).queryByRole('checkbox')).toBeNull();
  expect(within(row).getByText('PICK A FOLDER INSIDE YOUR HOME, NOT ALL OF IT')).toBeTruthy();
  expect(row.getAttribute('data-tickable')).toBe('false');
});

test('a drive root is refused with its own reason and never a silent no', () => {
  draw({ rows: [refusedRow(refusal('filesystem_root', null), '/')], ticked: new Set<string>() });
  expect(screen.getByText('A DRIVE ROOT IS NOT A PROJECT FOLDER')).toBeTruthy();
});

// §10.1b: the fourth is confirmable — over 500k estimated directories keeps its tick and opens
// a confirmation.
test('the confirmable refusal keeps its tick and asks', () => {
  const deps = draw({
    rows: [refusedRow(refusal('too_many_directories', 512_000), '/somewhere/big')],
    ticked: new Set(['/somewhere/big']),
    pendingConfirm: { pathDisplay: '/somewhere/big', estimatedDirs: 512_000 },
  });
  // The row *is* the control — `within` searches descendants, so the tick has to be read off
  // the row itself or the assertion passes on a row that has no control at all.
  const row = screen.getByTestId('fr-root-row');
  expect(row.tagName).toBe('BUTTON');
  expect(row.getAttribute('role')).toBe('checkbox');
  expect(row.getAttribute('aria-checked')).toBe('true');
  // The reason is a mis-pick, never slowness: 101k dirs/s makes 500,000 directories five seconds.
  expect(screen.getByText(/almost always a mis-pick/i)).toBeTruthy();
  expect(document.body.textContent).not.toMatch(/\bslow\b|\btoo long\b/i);
  fireEvent.click(screen.getByRole('button', { name: copy.CONFIRM_LARGE_LABEL }));
  expect(deps.onConfirmLarge).toHaveBeenCalledWith('/somewhere/big');
});

test('no confirmation is drawn when none is pending', () => {
  draw();
  expect(screen.queryByRole('button', { name: copy.CONFIRM_LARGE_LABEL })).toBeNull();
});

// §2.4: paths enter only from a native dialog owned by the shell. The screen asks for one; it
// never types one.
test('ADD A FOLDER asks the shell and sends no path', () => {
  const deps = draw();
  fireEvent.click(screen.getByRole('button', { name: copy.ADD_A_FOLDER_LABEL }));
  expect(deps.onAddFolder).toHaveBeenCalledTimes(1);
  // The handler is passed straight to onClick, so the only argument it can ever see is the
  // event — never a path this screen assembled.
  expect(deps.onAddFolder.mock.calls[0]?.filter((a) => typeof a === 'string')).toEqual([]);
});
