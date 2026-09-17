import type { CSSProperties, ReactElement, ReactNode, RefObject } from 'react';
import { useEffect, useRef, useState } from 'react';
import type { Problems, ScanStatus } from '../../generated/protocol.js';
import { hasReportableProblems } from '../notices/copy.js';
import { parseQuery } from '../../shared/query/parse.js';
import type { KeyAction, KeyEventLike } from '../keyboard/contexts.js';
import type { ShelfCounts } from './counts.js';
import { EmptyState, emptyStateModel, type LibraryPresence } from './EmptyState.js';
import { shelfKeyIntent } from './keyboard.js';
import type { Notice } from './notice.js';
import { NoticeSlot } from './NoticeSlot.js';
import type { ShelfPage } from './page.js';
import { fieldModel } from './QueryField.js';
import { TopBar } from './TopBar.js';
import type { ShelfView } from './viewState.js';

export const SHELF_SCROLL_CLASS = 'cdt-shelf-scroll';

/** Until the bar has been measured there is nothing to shed from: an unmeasured width must not
 *  read as a narrow one, or the first paint is a fully-shed bar in a wide window. */
const UNMEASURED_BAR_WIDTH = Number.POSITIVE_INFINITY;

export interface ShelfProps {
  readonly view: ShelfView;
  readonly page: ShelfPage;
  /** The library's own figures. The shelf holds a page, not the projection, so it cannot count
   *  them itself — and a zero it invented would be a figure with no owner on screen. */
  readonly counts: ShelfCounts;
  readonly notices: readonly Notice[];
  readonly scan: Pick<ScanStatus, 'running' | 'foundRepos' | 'problemCount'>;
  /**
   * §11.1's report, or `null` for *not read*. It decides whether the empty state offers a way
   * into the scan summary, because it is what that summary is drawn from.
   */
  readonly problems: Problems | null;
  /**
   * The inside of §8.0's box, for a section whose row is more than a string — §1.4's identity
   * card is a list of tickable rows. Undefined draws the generic notice, which is every other
   * kind; a supplier takes the body, the actions and the way out (`NoticeSlot`).
   */
  readonly renderNotice?: (notice: Notice, dismiss: () => void) => ReactNode;
  /** Three states, not two: see `LibraryPresence`. A shelf that has not read may not say the
   *  library is empty, and a boolean here is where that distinction used to die. */
  readonly library: LibraryPresence;
  /** Unix seconds, advanced by the owner. A frozen clock freezes every `as of` string below. */
  readonly now: number;
  /** True while a Peek is open. Owned by whoever mounts it; without it `Esc` declines, which is
   *  the correct answer for a shelf that has no Peek. */
  readonly peekOpen?: boolean;
  readonly onViewChange: (next: ShelfView) => void;
  /** Every claimed key action the shelf does not act on itself. Without a handler those keys
   *  are not claimed at all: preventing a default while nothing acts is a dead key. */
  readonly onKeyAction?: (action: KeyAction) => void;
  readonly onOpenPalette: () => void;
  readonly onOpenSettings: () => void;
  readonly onScan: () => void;
  readonly onOpenScanSummary: () => void;
  readonly onAddScanRoot: () => void;
  /**
   * §8.0's one scroll container, handed back so blocks 3 and 4 can virtualize against it. They
   * are `children` and cannot reach it otherwise, and a second scroller for the grid would make
   * the era headers scroll independently of the rows they head.
   */
  // React 19's `useRef<T>(null)` yields `RefObject<T | null>`: a ref genuinely is null before
  // mount, and the 18 types said otherwise. Widened rather than cast at the call site.
  readonly scrollRef?: RefObject<HTMLDivElement | null>;
  readonly children: ReactNode;
}

/** A real event, narrowed to what the key table reads. Never `null` for an element target: a
 *  table verified against a target the product never has is a table that proves nothing. */
function asKeyEvent(event: KeyboardEvent): KeyEventLike {
  const target = event.target;
  return {
    key: event.key,
    code: event.code,
    altKey: event.altKey,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    metaKey: event.metaKey,
    target:
      target instanceof HTMLElement
        ? { tagName: target.tagName, isContentEditable: target.isContentEditable }
        : null,
  };
}

/**
 * The four blocks and the one scroll container.
 *
 * Three blocks the design draws are not built — the status band, the quest strip and the
 * amnesty invitation — and **the space they leave is filled by nothing**: a level or ring
 * placeholder renders unknown as zero, and anything animated at rest costs frames the budget
 * puts at zero. The attention row moves up and the grid starts higher; that is the whole change.
 *
 * Blocks 3 and 4 arrive as `children`. This component decides only between that body and the
 * empty state, and hands down the one tile value the grid lays out against.
 */
export function Shelf(props: ShelfProps): ReactElement {
  const hostRef = useRef<HTMLDivElement>(null);
  const [barWidth, setBarWidth] = useState<number>(UNMEASURED_BAR_WIDTH);

  useEffect(() => {
    const host = hostRef.current;
    if (host === null) return;
    const measure = (): void => {
      setBarWidth(host.getBoundingClientRect().width || UNMEASURED_BAR_WIDTH);
    };
    measure();
    // jsdom has no ResizeObserver, and neither does an older shell; the window listener is a
    // fallback for the same measurement, not a second reading of it.
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', measure);
      return () => {
        window.removeEventListener('resize', measure);
      };
    }
    const observer = new ResizeObserver(measure);
    observer.observe(host);
    return () => {
      observer.disconnect();
    };
  }, []);

  const { view, onKeyAction, onOpenPalette } = props;
  const peekOpen = props.peekOpen ?? false;
  const selectedProjectId = view.selectedProjectId;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      const intent = shelfKeyIntent(asKeyEvent(event), {
        peekOpen,
        focusedProjectId: selectedProjectId,
      });
      // A decline is a key the shelf owns but will not act on now. It must reach the field.
      if (intent === null || intent.kind === 'decline') return;
      if (intent.action === 'quickSwitch') {
        if (intent.preventDefault) event.preventDefault();
        onOpenPalette();
        return;
      }
      if (onKeyAction === undefined) return;
      if (intent.preventDefault) event.preventDefault();
      onKeyAction(intent.action);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
    };
  }, [peekOpen, selectedProjectId, onKeyAction, onOpenPalette]);

  /** One place turns a control into a view. The query and its AST move together, so the pills
   *  and the filter cannot come from two different readings of one string. */
  const change = (next: Partial<ShelfView>): void => {
    const merged = { ...view, ...next };
    props.onViewChange(
      next.query === undefined ? merged : { ...merged, ast: parseQuery(next.query) },
    );
  };

  const queryRanAndMatchedNothing = props.page.matched === 0 && view.query.length > 0;
  const showEmpty = props.library !== 'present' || queryRanAndMatchedNothing;
  // From the report itself, never from `scan.status`: the panel this link opens is drawn from
  // the report, so offering the link on a different reading is how it came to open nothing.
  const hadProblems = hasReportableProblems(props.problems);

  const scrollStyle = { '--cdt-tile': `${String(view.density)}px` } as CSSProperties;

  return (
    <div className="cdt-shelf" ref={hostRef}>
      <TopBar
        view={view}
        field={fieldModel(view.query, view.ast, [])}
        scan={props.scan}
        barWidth={barWidth}
        onQueryChange={(query) => {
          change({ query });
        }}
        onSortChange={(sort) => {
          change({ sort });
        }}
        onDensityChange={(density) => {
          change({ density });
        }}
        onViewModeChange={(viewMode) => {
          change({ viewMode });
        }}
        onScan={props.onScan}
        onOpenScanSummary={props.onOpenScanSummary}
        onOpenPalette={props.onOpenPalette}
        onOpenSettings={props.onOpenSettings}
      />
      <div className={SHELF_SCROLL_CLASS} style={scrollStyle} ref={props.scrollRef}>
        <NoticeSlot
          candidates={props.notices}
          dismissed={view.dismissedNotices}
          onDismiss={(key) => {
            change({ dismissedNotices: [...view.dismissedNotices, key] });
          }}
          {...(props.renderNotice === undefined ? {} : { renderContent: props.renderNotice })}
        />
        {showEmpty ? (
          <EmptyState
            model={emptyStateModel({
              ast: props.page.ast,
              counts: props.counts,
              library: props.library,
              now: props.now,
            })}
            onClearQuery={() => {
              change({ query: '' });
            }}
            onAddScanRoot={props.onAddScanRoot}
            onOpenScanSummary={hadProblems ? props.onOpenScanSummary : null}
          />
        ) : (
          props.children
        )}
      </div>
    </div>
  );
}
