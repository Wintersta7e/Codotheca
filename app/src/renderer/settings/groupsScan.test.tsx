import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Root, RootId, TargetId, TargetList } from '../../generated/protocol.js';
import {
  CONTENT_SCAN_CONSEQUENCE,
  CONTENT_SCAN_LABEL,
  CONTENT_SCAN_LANGUAGES,
} from '../../shared/contentScan.js';
import { EXCLUSION_CAPTION, EXCLUSION_LIST } from '../../shared/skipList.js';
import { LOCKFILE_NAMES } from '../../shared/lockfileNames.js';
import { CONSENT_PARAGRAPH } from '../firstrun/copy.js';
import {
  EXCLUSION_PRIVACY_CAPTION,
  LAUNCH_TARGET_LANGUAGES,
  SCAN_GROUP_ROWS,
  ScanGroups,
  SCANNING_STATEMENTS,
  TARGET_FOOTNOTE,
  rootsCaption,
} from './groupsScan.js';

afterEach(cleanup);

const root = (over: Partial<Root> = {}): Root => ({
  id: 1 as unknown as RootId,
  pathDisplay: '<home>/code',
  kind: 'linux',
  distro: '',
  enabled: true,
  descendIntoRepos: false,
  provenance: 'convention',
  state: 'watched',
  addedAt: 100,
  projectCount: 12,
  ...over,
});

const targets = (verifyState: TargetList['rows'][number]['verifyState']): TargetList => ({
  resolved: null,
  rows: [
    {
      id: 1 as unknown as TargetId,
      kind: 'editor',
      name: 'An editor',
      projectId: null,
      locationId: null,
      language: 'RS',
      sortIndex: 0,
      detected: true,
      verifyState,
      verifiedAt: null,
      execDisplay: '<apps>/an-editor',
    },
  ],
});

const props = {
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

describe('group 1, SCAN ROOTS', () => {
  it('reads — when the root list is unavailable, never 0 OF 0 ACTIVE', () => {
    // §7.7a: unknown is never rendered as zero. A configured machine reading `0 OF 0 ACTIVE`
    // teaches itself as an unconfigured one.
    expect(rootsCaption(null)).toBe('—');
    expect(rootsCaption([])).toBe('0 OF 0 ACTIVE');
    expect(rootsCaption([root(), root({ id: 2 as unknown as RootId, enabled: false })])).toBe(
      '1 OF 2 ACTIVE',
    );
    render(<ScanGroups {...props} />);
    expect(screen.queryByText(/OF 0 ACTIVE/)).toBeNull();
  });

  it('draws no control that removes a root, in any spelling', () => {
    render(<ScanGroups {...props} roots={[root()]} />);
    for (const banned of ['REMOVE', 'DELETE', 'UNINSTALL']) {
      expect(screen.queryByRole('button', { name: new RegExp(banned, 'i') })).toBeNull();
    }
    // §17: disabling is the reversible form and is the only one drawn.
    expect(screen.getByRole('switch', { name: '<home>/code' })).toBeTruthy();
  });

  it('names each root switch after its own root, so two roots are not one name', () => {
    render(
      <ScanGroups
        {...props}
        roots={[root(), root({ id: 2 as unknown as RootId, pathDisplay: '<home>/scratch' })]}
      />,
    );
    expect(screen.getAllByRole('switch', { name: /Descend into repositories/ })).toHaveLength(2);
    expect(
      screen.getByRole('switch', { name: 'Descend into repositories under <home>/scratch' }),
    ).toBeTruthy();
  });

  it('toggles a root by id, and reports the value it is moving to', () => {
    const onSetEnabled = vi.fn();
    render(<ScanGroups {...props} roots={[root()]} onSetEnabled={onSetEnabled} />);
    fireEvent.click(screen.getByRole('switch', { name: '<home>/code' }));
    expect(onSetEnabled).toHaveBeenCalledWith(1, false);
  });

  it('never prints an uncounted root as zero projects', () => {
    render(<ScanGroups {...props} roots={[root({ projectCount: null })]} />);
    expect(screen.queryByText(/0 PROJECTS/)).toBeNull();
    cleanup();
    render(<ScanGroups {...props} roots={[root({ projectCount: 12 })]} />);
    expect(screen.getByText(/12 PROJECTS/)).toBeTruthy();
  });

  it('a folder is added through the shell dialog, never a typed path', () => {
    render(<ScanGroups {...props} />);
    // §2.4: no field on this surface accepts a path.
    expect(screen.queryByRole('textbox')).toBeNull();
    const spec = SCAN_GROUP_ROWS.find((r) => r.id === 'roots-add');
    expect(spec?.backing).toEqual({ kind: 'shell', channel: 'codotheca:pick-root' });
  });

  it('RESCAN NOW issues scan.start and nothing else', () => {
    const onRescan = vi.fn();
    render(<ScanGroups {...props} onRescan={onRescan} />);
    fireEvent.click(screen.getByRole('button', { name: 'RESCAN NOW' }));
    expect(onRescan).toHaveBeenCalledTimes(1);
    expect(SCAN_GROUP_ROWS.find((r) => r.id === 'roots-rescan')?.backing).toEqual({
      kind: 'command',
      command: 'scan.start',
    });
  });
});

describe('group 2, LAUNCH TARGETS', () => {
  it('shows a row per language and the footnote verbatim', () => {
    // Scoped to this group: §29.8's grant renders its own language list in group 4, and two
    // groups naming `C++` is not two rows in this one.
    const { container } = render(<ScanGroups {...props} />);
    const group = container.querySelector('[data-group="targets"]');
    expect(group).toBeTruthy();
    for (const tag of LAUNCH_TARGET_LANGUAGES) {
      expect(within(group as HTMLElement).getByText(tag)).toBeTruthy();
    }
    expect(
      screen.getByText(
        'A project can override its own target on its page. Whichever you pick, the launch is recorded — that is what playtime counts.',
      ),
    ).toBeTruthy();
    expect(TARGET_FOOTNOTE).toContain('playtime counts');
  });

  it('an unverified target says so rather than claiming a currency it does not have', () => {
    render(<ScanGroups {...props} targets={targets('unverified')} />);
    // §11.5: unverified is not `ok`. The row states what is known about it and never OK.
    expect(screen.queryByText('OK')).toBeNull();
    expect(screen.getByText('NOT CHECKED YET')).toBeTruthy();
    expect(screen.getByText('<apps>/an-editor')).toBeTruthy();
  });

  it('a verified target says nothing, because there is nothing to warn about', () => {
    render(<ScanGroups {...props} targets={targets('ok')} />);
    expect(screen.queryByText('NOT CHECKED YET')).toBeNull();
    expect(screen.queryByText('NOT FOUND WHERE IT WAS')).toBeNull();
  });

  it('distinguishes a language with no target from a list never read', () => {
    render(<ScanGroups {...props} targets={targets('ok')} />);
    // Five of the six tags have no row in this list; none of them may read as configured.
    expect(screen.getAllByText('NOT SET')).toHaveLength(LAUNCH_TARGET_LANGUAGES.length - 1);
    cleanup();
    render(<ScanGroups {...props} targets={null} />);
    expect(screen.queryByText('NOT SET')).toBeNull();
  });

  it('draws CHANGE only when a picker exists behind it', () => {
    render(<ScanGroups {...props} targets={targets('ok')} />);
    expect(screen.queryByRole('button', { name: 'CHANGE' })).toBeNull();
    cleanup();
    const chooseLaunchTarget = vi.fn();
    render(<ScanGroups {...props} targets={targets('ok')} slots={{ chooseLaunchTarget }} />);
    const buttons = screen.getAllByRole('button', { name: 'CHANGE' });
    expect(buttons).toHaveLength(LAUNCH_TARGET_LANGUAGES.length);
    fireEvent.click(buttons[0] as HTMLElement);
    expect(chooseLaunchTarget).toHaveBeenCalledWith('RS');
  });
});

describe('group 3, EXCLUDED FROM EVERY SCAN', () => {
  it('renders §4.3 list from the one shared constant and offers no + ADD', () => {
    render(<ScanGroups {...props} />);
    for (const entry of EXCLUSION_LIST) expect(screen.getByText(entry)).toBeTruthy();
    // No command writes a user skip entry and `SettingsPatch` has no field for one, so the
    // design's dashed `+ ADD` is a dead switch and is cut.
    expect(screen.queryByRole('button', { name: /\+\s*ADD/ })).toBeNull();
  });

  it('carries §11.3a caption, not the first-run one that promises an edit', () => {
    render(<ScanGroups {...props} />);
    expect(screen.getByText(EXCLUSION_PRIVACY_CAPTION)).toBeTruthy();
    // §10.1b's caption says the list is *editable in settings*. This is settings, and the
    // control that would edit it does not exist, so that sentence would be false here.
    expect(EXCLUSION_PRIVACY_CAPTION).not.toBe(EXCLUSION_CAPTION);
    expect(screen.queryByText(EXCLUSION_CAPTION)).toBeNull();
  });
});

describe('group 4, SCANNING · HOW IT WORKS', () => {
  it('carries the corrected statements and none of the superseded ones', () => {
    render(<ScanGroups {...props} />);
    // §29.8 retired *"Source files are never read"*: J7 reads them once the user says so, and a
    // statement that is false for the user who said yes is what this file exists to prevent.
    expect(screen.queryByText('Source files are never read')).toBeNull();
    expect(
      screen.getByText('Names and timestamps, plus README, LICENSE and manifests at the root'),
    ).toBeTruthy();
    expect(
      screen.getByText('Your scan roots, and the projects you opened most recently'),
    ).toBeTruthy();
    expect(screen.queryByText(/File contents are read only for debt items/)).toBeNull();
    // This string is false against §6, in the one group §11.3a calls the boundary.
    expect(screen.queryByText('Nothing is watched inside a repository')).toBeNull();
    expect(screen.queryByText(/Only the thirty most recent projects are watched/)).toBeNull();
  });

  it('draws every statement as a statement, so only the grant announces as a control', () => {
    const { container } = render(<ScanGroups {...props} />);
    const statements = SCAN_GROUP_ROWS.filter(
      (r) => r.group === 'scanning' && r.backing.kind === 'statement',
    );
    expect(statements.length).toBeGreaterThan(0);
    for (const spec of statements) {
      const row = container.querySelector(`[data-row="${spec.id}"]`);
      expect(row?.querySelector('button, [role="switch"], [tabindex]')).toBeNull();
    }
    // §29.8's grant is the one row in this group that does something.
    const grant = SCAN_GROUP_ROWS.filter((r) => r.id === 'content-scan');
    expect(grant).toHaveLength(1);
    expect(grant[0]?.backing).toEqual({ kind: 'command', command: 'settings.set' });
  });

  it('the grant is a real switch, and it reads nothing until settings have been read', () => {
    const onSetContentScan = vi.fn();
    const { container } = render(
      <ScanGroups {...props} contentScanEnabled={null} onSetContentScan={onSetContentScan} />,
    );
    // `null` is *not read*, never *off*: an unread setting drawn as off invites a click that
    // would set a value the user never saw the current state of.
    expect(container.querySelector('[data-row="content-scan"]')).toBeNull();

    cleanup();
    render(
      <ScanGroups {...props} contentScanEnabled={false} onSetContentScan={onSetContentScan} />,
    );
    const grant = screen.getByRole('switch', { name: CONTENT_SCAN_LABEL });
    expect(grant.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(grant);
    expect(onSetContentScan).toHaveBeenCalledWith(true);
    expect(screen.getByText(CONTENT_SCAN_CONSEQUENCE)).toBeTruthy();
    const scanning = document.querySelector('[data-group="scanning"]');
    expect(scanning).toBeTruthy();
    for (const language of CONTENT_SCAN_LANGUAGES) {
      expect(within(scanning as HTMLElement).getByText(language)).toBeTruthy();
    }
  });
});

describe('[p3] §32.6: the drawer names the lock files the read opens', () => {
  it('says the same thing as the first-run paragraph, from the same list', () => {
    const row = SCANNING_STATEMENTS.find((s) => s.label.includes('lock files'));
    expect(row, 'the scan group states no lock file row').toBeDefined();
    const said = `${row?.label ?? ''} ${row?.note ?? ''}`.toLowerCase();
    for (const name of LOCKFILE_NAMES) {
      expect(said, name).toContain(name.toLowerCase());
    }
    expect(LOCKFILE_NAMES.length, 'a loop over no names proves nothing').toBe(6);
    expect(said).toContain('three directories deep');
    expect(said).toContain('16 mb each');

    // **The two surfaces cannot disagree**: §11.3 group 4 has three sites for this claim, and a
    // change that moved one would leave the product contradicting itself in its own drawer.
    for (const name of LOCKFILE_NAMES) {
      expect(CONSENT_PARAGRAPH, name).toContain(name);
    }
    expect(CONSENT_PARAGRAPH).toContain('three directories deep');
    expect(CONSENT_PARAGRAPH).toContain('16 MB each');
  });

  it("leaves J6's own 256 KB cap alone", () => {
    const named = SCANNING_STATEMENTS.find((s) => s.note.includes('256 KB EACH'));
    expect(named, "J6's named-file row lost its own cap").toBeDefined();
  });
});
