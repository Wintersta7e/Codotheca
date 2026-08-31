import './fonts';
import './styles/tokens.css';
import './styles/base.css';
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
