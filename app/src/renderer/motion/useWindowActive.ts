import { useEffect, useState } from 'react';

/**
 * Focused **and** visible. §11.6 suspends the scheduled flicker while unfocused or hidden with
 * no queued backlog, and §7.8 schedules the live tile's timer only while the window is visible;
 * both read this one value so they cannot disagree about what "idle" means.
 */
function currentlyActive(): boolean {
  const visible = document.visibilityState !== 'hidden';
  return visible && document.hasFocus();
}

export function useWindowActive(): boolean {
  const [active, setActive] = useState(currentlyActive);
  useEffect(() => {
    const update = (): void => {
      setActive(currentlyActive());
    };
    window.addEventListener('focus', update);
    window.addEventListener('blur', update);
    document.addEventListener('visibilitychange', update);
    update();
    return () => {
      window.removeEventListener('focus', update);
      window.removeEventListener('blur', update);
      document.removeEventListener('visibilitychange', update);
    };
  }, []);
  return active;
}
