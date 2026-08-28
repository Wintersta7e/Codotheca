import { expect, it } from 'vitest';

it('has a DOM, which is what plan 12 mounts components into', () => {
  document.body.innerHTML = '<main id="shelf"></main>';
  expect(document.querySelector('#shelf')).not.toBeNull();
});
