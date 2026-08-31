import type { ProjectId, ProjectRow } from '../../generated/protocol.js';
import type { ConditionOrNull, ShelfRow } from './row.js';
import { toShelfRow } from './row.js';

// No re-export of `row.ts`'s names. `ShelfRow`, `toShelfRow` and `projectionCapabilities` are
// imported from `./row.js` by every consumer — this plan's later tasks, and 13b/13c's components
// — so there is one import path per name and no second place for the row shape to appear to live.

export interface FlagDelta {
  readonly isPinned: boolean;
  readonly isArchived: boolean;
  readonly isHidden: boolean;
}

/**
 * The resident §8.3 projection. Insertion order is stable so a scan appends rather than
 * reshuffling; ordering for display is the shelf page's job, not this store's.
 */
export class ProjectionStore {
  #rows = new Map<ProjectId, ShelfRow>();
  #generation = 0;
  #listeners = new Set<() => void>();

  get generation(): number {
    return this.#generation;
  }

  get rows(): readonly ShelfRow[] {
    return [...this.#rows.values()];
  }

  get(id: ProjectId): ShelfRow | undefined {
    return this.#rows.get(id);
  }

  applySnapshot(rows: readonly ProjectRow[], generation: number): void {
    this.#rows = new Map(rows.map((row) => [row.id, toShelfRow(row)]));
    this.#generation = generation;
    this.#notify();
  }

  upsert(row: ProjectRow): void {
    this.#rows.set(row.id, toShelfRow(row));
    this.#notify();
  }

  merge(from: ProjectId, into: ProjectId): void {
    if (!this.#rows.has(from)) return;
    this.#rows.delete(from);
    if (!this.#rows.has(into)) return;
    this.#notify();
  }

  setFlags(id: ProjectId, flags: FlagDelta): void {
    const existing = this.#rows.get(id);
    if (existing === undefined) return;
    this.#rows.set(id, { ...existing, ...flags });
    this.#notify();
  }

  setCondition(id: ProjectId, conditionSignal: ConditionOrNull): void {
    const existing = this.#rows.get(id);
    if (existing === undefined) return;
    this.#rows.set(id, { ...existing, conditionSignal });
    this.#notify();
  }

  subscribe(listener: () => void): () => void {
    this.#listeners.add(listener);
    return () => {
      this.#listeners.delete(listener);
    };
  }

  #notify(): void {
    for (const listener of this.#listeners) listener();
  }
}
