import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import type { Mock } from 'vitest';
import { ResidencyCard, residencySettingsArgs } from './ResidencyCard';
import { RESIDENCY_ANSWERS, residencyAutostart } from './notices';
import { isNewArrival } from './newArrivals';
import type { ProjectId } from '../../generated/protocol';
import * as copy from './copy';

afterEach(cleanup);

function renderCard(): { onAnswer: Mock<(answer: 'startWithTheSystem' | 'leaveItOff') => void> } {
  const onAnswer = vi.fn<(answer: 'startWithTheSystem' | 'leaveItOff') => void>();
  render(<ResidencyCard onAnswer={onAnswer} />);
  return { onAnswer };
}

// §11.3, measured: the three residency figures and the two latency figures, all five, plus the
// window lifetime the trade rests on.
test('the card carries the measured figures', () => {
  renderCard();
  const text = document.body.textContent ?? '';
  for (const figure of ['307 MB', '522 MB', '232 MB', '9 ms', '134', 'thirty minutes']) {
    expect(text).toContain(figure);
  }
});

// "~30 MB" was false by seventeen times and was once written into consent copy.
test('the figure that was false by seventeen times appears nowhere', () => {
  renderCard();
  const text = document.body.textContent ?? '';
  expect(text).not.toMatch(/~\s*30\s*MB/i);
  expect(text).not.toMatch(/\b30\s*MB\b/i);
  // The guard above must not be passing because the real figures are absent.
  expect(text).toContain('307 MB');
});

test('the title and the two answers are the whole control surface', () => {
  renderCard();
  expect(screen.getByText(copy.RESIDENCY_TITLE)).toBeTruthy();
  expect(screen.getAllByRole('button')).toHaveLength(2);
  // §11.3a group 6 owns the real switch; this notice is two answers and no control.
  expect(document.querySelectorAll('input, select')).toHaveLength(0);
});

// §11.3a: the stamp fires on either answer, so neither may be a no-op.
test('both answers answer', () => {
  const { onAnswer } = renderCard();
  fireEvent.click(screen.getByRole('button', { name: copy.RESIDENCY_YES }));
  expect(onAnswer).toHaveBeenLastCalledWith('startWithTheSystem');
  fireEvent.click(screen.getByRole('button', { name: copy.RESIDENCY_NO }));
  expect(onAnswer).toHaveBeenLastCalledWith('leaveItOff');
  expect(onAnswer).toHaveBeenCalledTimes(2);
});

test('the secondary answer does not promise a later ask', () => {
  renderCard();
  expect(screen.queryByRole('button', { name: 'NOT NOW' })).toBeNull();
  expect(screen.queryByRole('button', { name: /LATER/i })).toBeNull();
});

// ---------------------------------------------------------------------------
// The chain, end to end. Three things must be true together and each was in a
// different plan: the card sends an `autostart` patch, `settings.set` stamps on
// it, and first run exposes the stamp. Any one missing and the `NEW` chip is
// silently dead — it does not fail, it never appears.
// ---------------------------------------------------------------------------

const STAMP = 1_770_000_000;

test('every way out of the row produces a patch that carries autostart', () => {
  for (const answer of RESIDENCY_ANSWERS) {
    const args = residencySettingsArgs(answer);
    // The core keys the stamp on the field being *present*, not on its value.
    expect(args.patch.autostart).not.toBeNull();
    expect(args.patch.autostart).toBe(residencyAutostart(answer));
  }
});

// The envelope as well as the patch: `settings.set` takes `{ patch }`, and a well-formed patch
// under the wrong key is refused by the core with `unknown field`, which reads as a schema bug.
test('the call names autostart under the key the command takes, and disturbs nothing else', () => {
  expect(residencySettingsArgs('leaveItOff')).toEqual({
    patch: {
      effectsTier: null,
      reducedMotionOverride: null,
      autostart: false,
      residentShortcut: null,
      roastEnabled: null,
      logLevel: null,
    },
  });
});

// Declining is still an answer, and this is the whole reason it must send the patch: the chip
// is dead until the stamp lands, and a card that closed silently would leave it dead forever.
test('declining the ask still arms the NEW chip', () => {
  const { onAnswer } = renderCard();
  fireEvent.click(screen.getByRole('button', { name: copy.RESIDENCY_NO }));
  const answer = onAnswer.mock.calls[0]?.[0];
  expect(answer).toBe('leaveItOff');

  const arrival = { id: 1 as ProjectId, createdAt: STAMP + 60, acknowledgedAt: null };
  // Before the answer the stamp is NULL, so nothing on the shelf can be new.
  expect(isNewArrival(arrival, null)).toBe(false);

  // The patch the answer produces is what `settings.set` stamps on, and the stamp is what the
  // snapshot hands back as `firstRunCompletedAt`.
  const args = residencySettingsArgs(answer ?? 'leaveItOff');
  const firstRunCompletedAt = args.patch.autostart === null ? null : STAMP;
  expect(isNewArrival(arrival, firstRunCompletedAt)).toBe(true);
});
