import type { CommandName, SessionRef, TargetVerification, Topic } from '../../generated/protocol';
import { focusArgs } from '../../shared/sessionFocus';
import type { TopicHandler } from '../core/client';
import type { JoinStep } from '../startup';

export interface LaunchStepDeps {
  readonly request: (name: CommandName, args: unknown) => Promise<unknown>;
  readonly subscribe: (topic: Topic, handler: TopicHandler) => () => void;
  readonly onRecovered: (sessions: readonly SessionRef[]) => void;
  readonly onVerified: (rows: readonly TargetVerification[]) => void;
}

/**
 * §11.2's core lane closes orphaned sessions before the join. The closure itself runs in the
 * core before its command loop accepts anything, which is stronger than this step could be —
 * so this step does the two things only the shell can: it subscribes before issuing, so a
 * `session/ended` from recovery is not lost, and it establishes the join-time focus level,
 * which is nothing, because the window has only just opened.
 */
export function sessionRecoveryStep(deps: LaunchStepDeps): JoinStep {
  return {
    name: 'session-recovery',
    run: async () => {
      const recovered: SessionRef[] = [];
      const unsubscribe = deps.subscribe('session', {
        onEvent: (event, data) => {
          if (event !== 'ended') return;
          const session = (data as { session?: SessionRef } | null)?.session;
          if (session) recovered.push(session);
        },
        onSnapshot: () => undefined,
      });
      try {
        // A reply proves recovery is done: the core answers no command until it has run.
        await deps.request('session.focus', focusArgs(null));
      } finally {
        unsubscribe();
      }
      deps.onRecovered(recovered);
    },
  };
}

/**
 * The launch lane's steps, in the order §11.2 and §4bis.5 put them: recovery first, because a
 * session orphaned by a crash must be closed before anything reads playtime, then the target
 * verification sweep, which is a repair pass nothing blocks on.
 *
 * **`verifyTargets` is injected, not declared.** The step is plan 17's
 * (`app/src/main/joinSteps.ts`), which has not landed yet; declaring a second one here would
 * be the duplicate C6 exists to prevent. Until it lands the lane is honestly one step short,
 * and `LaunchStepDeps.onVerified` is already the shape plan 17's factory takes.
 */
export function launchJoinSteps(
  deps: LaunchStepDeps,
  verifyTargets?: JoinStep,
): readonly JoinStep[] {
  const recovery = sessionRecoveryStep(deps);
  return verifyTargets === undefined ? [recovery] : [recovery, verifyTargets];
}
