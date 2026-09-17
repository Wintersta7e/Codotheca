import './fonts';
import './styles/tokens.css';
import './styles/base.css';
// Between base and motion, so §11.6's tier clamps still win on order as well as specificity.
import './styles/card.css';
import './styles/projectPage.css';
import './shelf/shelf.css';
// §8.5.1's gesture spans the shelf and the page, so it sits after both and before the tier clamp.
import './styles/transition.css';
// §11.2a: a first-run beat unmounts the shelf, so these rules never coexist with the grid — but
// the sheet is loaded with the rest, because only `main.tsx` mounts CSS in this renderer.
import './firstrun/firstRun.css';
// motion.css stays last: it clamps by effects tier and has to win over every sheet above it.
import './styles/motion.css';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import { applyEffectsTier } from './effectsTier';

// Before anything mounts. At `off` the GPU may already be disabled in the shell, and the
// document must agree with that decision on the first frame rather than the second.
applyEffectsTier(document.documentElement, window.codotheca.effectsTier);

const host = document.getElementById('root');
if (host === null) {
  throw new Error('renderer root element is missing');
}
createRoot(host).render(<App />);
